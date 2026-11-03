package com.goatpacker.web.service.runner

import com.fasterxml.jackson.databind.ObjectMapper
import com.goatpacker.web.config.PlatformProperties
import com.goatpacker.web.config.SlicerKind
import com.goatpacker.web.model.Job
import com.goatpacker.web.util.ProcessKiller
import com.goatpacker.web.util.ShellChain
import org.apache.commons.compress.archivers.tar.TarArchiveInputStream
import org.slf4j.LoggerFactory
import org.springframework.context.annotation.Profile
import org.springframework.stereotype.Component
import java.nio.file.Files
import java.nio.file.LinkOption
import java.nio.file.Path
import java.util.concurrent.CompletableFuture
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.TimeUnit

/**
 * Only the trusted worker controls Docker. Untrusted code receives a bounded tmpfs, not a
 * writable host mount, network, daemon socket, credentials, or another job's directories.
 */
@Component
@Profile("server")
class NativeLauncher(
    private val props: PlatformProperties,
    private val json: ObjectMapper,
    private val docker: DockerCli,
) {
    private val log = LoggerFactory.getLogger(javaClass)
    private val containers = ConcurrentHashMap<String, String>()
    private val volumes = ConcurrentHashMap<String, String>()
    private val images = ConcurrentHashMap<String, String>()

    /** Проверяет образы раннеров и возвращает их неизменяемые id по виду слайсера; в local-режиме образов нет. */
    fun qualify(): Map<String, String> {
        if (props.localRunner) {
            check(props.allowLocalRunner && !props.production)
            return emptyMap()
        }
        return SlicerKind.ALL.associateWith { kind ->
            val ref = props.runnerImage(kind)
            check(ref.isNotBlank()) { "Runner image missing: $kind" }
            val id = docker.run(listOf("docker", "image", "inspect", "--format", "{{.Id}}", ref)).trim()
            check(id.startsWith("sha256:"))
            id
        }
    }

    fun hostIdentity(): String = if (props.localRunner) "local" else docker.run(listOf("docker", "info", "--format", "{{.ID}}")).trim()

    /** Убирает контейнеры и тома, оставшиеся от прошлой загрузки этого исполнителя. */
    fun recoverNode() {
        if (props.localRunner) return
        val workerLabel = "label=$WORKER_LABEL=${props.workerId}"
        for (id in docker.run(listOf("docker", "ps", "-aq", "--filter", workerLabel)).lines().filter(String::isNotBlank)) {
            docker.forceRemove(id)
        }
        for (volume in docker.run(listOf("docker", "volume", "ls", "-q", "--filter", workerLabel)).lines().filter(String::isNotBlank)) {
            docker.run(listOf("docker", "volume", "rm", volume))
        }
    }

    /** [prelude] — подготовительная команда (source_project для STL) в том же процессе, см. [ShellChain]. */
    fun start(job: Job, command: List<String>, prelude: List<String>? = null): Process =
        if (props.localRunner) startLocal(job, command, prelude) else startInContainer(job, command, prelude)

    /** Freeze all descendants before reading files, so export has no symlink/size TOCTOU. */
    fun collect(job: Job) {
        if (props.localRunner) return
        val container = containers[job.id] ?: error("Runner not tracked")
        docker.run(listOf("docker", "pause", container))
        var remaining = props.outputReservationBytes
        for (name in EXPORTED_FILES) {
            val target = job.directory.resolve(name)
            Files.createDirectories(target.parent)
            remaining -= exportFile(job, name, target, remaining)
        }
        // Кэш проб не результат: его сбой не должен ронять джоб, а при ошибке пробы просто посчитаются заново.
        runCatching { exportCache(job) }.onFailure { log.warn("event=probe_cache_export_failed jobId={} reason={}", job.id, it.message) }
        stop(job)
    }

    fun stop(job: Job) {
        // An export helper is trusted but may survive a disconnected Docker CLI.
        if (!props.localRunner && (containers.containsKey(job.id) || volumes.containsKey(job.id))) {
            docker.forceRemove("goat-export-${job.id}")
        }
        containers[job.id]?.let { id ->
            docker.forceRemove(id)
            containers.remove(job.id, id)
        }
        volumes[job.id]?.let { volume ->
            docker.run(listOf("docker", "volume", "rm", volume))
            volumes.remove(job.id, volume)
        }
        images.remove(job.id)
    }

    fun tracked(job: Job): Boolean = containers.containsKey(job.id)

    /** Останавливает контейнер и процесс джоба; false — остановку подтвердить не удалось (слот освобождать нельзя). */
    fun terminate(job: Job): Boolean = runCatching {
        stop(job)
        job.process?.let(ProcessKiller::kill)
        true
    }.getOrElse {
        log.error("event=runner_cleanup_failed jobId={} reason={}", job.id, it.message)
        false
    }

    private fun startLocal(job: Job, command: List<String>, prelude: List<String>?): Process {
        check(props.allowLocalRunner && !props.production) { "Local runner forbidden in production" }
        val builder = ProcessBuilder(ShellChain.command(prelude, command)).directory(job.directory.toFile()).redirectErrorStream(true)
        builder.environment()["RAYON_NUM_THREADS"] = rayonThreads()
        // Как docker-ветка (--env GOAT_LOG_FORMAT=json): stage/layout-события нужны трекеру.
        builder.environment()["GOAT_LOG_FORMAT"] = "json"
        return builder.start()
    }

    private fun startInContainer(job: Job, command: List<String>, prelude: List<String>?): Process {
        val kind = json.readTree(job.directory.resolve(JobArtifacts.PROFILE_FILE).toFile()).path("slicer_kind").asText()
        val image = job.runtimeImage ?: props.runnerImage(kind)
        check(image.isNotBlank())
        val name = "goat-${job.id}-${job.runtimeFence}"
        val volume = "$name-work"
        docker.run(createVolumeCommand(volume))
        volumes[job.id] = volume
        images[job.id] = image
        try {
            val container = docker.run(createContainerCommand(job, name, volume, image)).trim()
            containers[job.id] = container
            docker.run(listOf("docker", "start", container))
            copyInputs(job, container)
            // Independent timeout survives a disconnected JVM or a dead control plane.
            val inContainer = listOf("docker", "exec", container, "/usr/bin/timeout", "--signal=TERM", "--kill-after=5s", job.runtimeTimeoutSeconds.toString()) +
                ShellChain.command(prelude?.let(::inImage), inImage(command))
            return ProcessBuilder(inContainer).redirectErrorStream(true).start()
        } catch (ex: Exception) {
            stop(job)
            throw ex
        }
    }

    private fun createVolumeCommand(volume: String) = listOf(
        "docker", "volume", "create", "--label", "$WORKER_LABEL=${props.workerId}", "--driver", "local",
        "--opt", "type=tmpfs", "--opt", "device=tmpfs",
        "--opt", "o=size=${props.scratchLimitBytes},uid=$RUNNER_UID,gid=$RUNNER_UID,mode=0700,nosuid,nodev",
        volume,
    )

    private fun createContainerCommand(job: Job, name: String, volume: String, image: String) = listOf(
        "docker", "create", "--platform", PLATFORM, "--name", name,
        "--label", "$WORKER_LABEL=${props.workerId}", "--label", "goat.job=${job.id}", "--label", "goat.fence=${job.runtimeFence}",
        "--network", "none", "--read-only", "--cap-drop", "ALL", "--security-opt", "no-new-privileges",
        "--pids-limit", "256", "--cpus", props.workerCpus.toString(),
        "--memory", "${props.workerMemoryMib}m", "--memory-swap", "${props.workerMemoryMib}m",
        "--user", "$RUNNER_UID:$RUNNER_UID", "--init",
        "--tmpfs", "/tmp:rw,noexec,nosuid,nodev,size=64m",
        "--mount", "type=volume,src=$volume,dst=$WORK_DIR,volume-nocopy",
        "--workdir", WORK_DIR, "--env", "HOME=$WORK_DIR/home", "--env", "RAYON_NUM_THREADS=${rayonThreads()}",
        "--env", "GOAT_LOG_FORMAT=json", image, "/usr/bin/sleep", "infinity",
    )

    /** Вход, профиль и (для доводки) состояние упаковки едут в контейнер по stdin: на хост ничего не монтируется. */
    private fun copyInputs(job: Job, container: String) {
        // STL-вход едет как input.stl: input.3mf из него соберёт source_project внутри контейнера.
        val input = if (isRegularFile(job.directory.resolve(INPUT_STL))) INPUT_STL else INPUT_3MF
        // Доводка «Нарезать стол» везёт ещё и состояние упаковки родителя.
        val finishState = JobArtifacts.FINISH_STATE.takeIf { isRegularFile(job.directory.resolve(it)) }
        // Кэш проб владельца (его кладёт воркер до запуска): имена из белого списка, в команду идут без экранирования.
        val cache = job.directory.resolve(ProbeCacheEntries.DIR).takeIf { Files.isDirectory(it, LinkOption.NOFOLLOW_LINKS) }
            ?.let { dir -> Files.list(dir).use { stream -> stream.toList() } }.orEmpty()
            .filter { ProbeCacheEntries.isEntryName(it.fileName.toString()) && isRegularFile(it) }
            .map { "${ProbeCacheEntries.DIR}/${it.fileName}" }
        for (file in listOfNotNull(input, JobArtifacts.PROFILE_FILE, finishState) + cache) {
            val source = job.directory.resolve(file)
            check(isRegularFile(source))
            val transfer = ProcessBuilder("docker", "exec", "-i", container, "/bin/sh", "-c", "mkdir -p $WORK_DIR/${file.substringBeforeLast('/', ".")} && cat > $WORK_DIR/$file")
                .redirectInput(source.toFile())
                .redirectOutput(ProcessBuilder.Redirect.DISCARD)
                .redirectError(ProcessBuilder.Redirect.DISCARD)
                .start()
            if (!transfer.waitFor(TRANSFER_TIMEOUT_SECONDS, TimeUnit.SECONDS)) {
                transfer.destroyForcibly()
                error("Input transfer timed out")
            }
            check(transfer.exitValue() == 0) { "Input transfer failed" }
        }
    }

    /** Бинарник в образе лежит в /usr/local/bin под своим именем; путь хоста там не существует. */
    private fun inImage(command: List<String>) = listOf("$IMAGE_BIN_DIR/${Path.of(command.first()).fileName}") + command.drop(1)

    /**
     * Копирует один файл из замороженного тома через одноразовый вспомогательный контейнер и tar:
     * ровно один обычный файл не больше [limit] байт, иначе исключение. Возвращает размер файла.
     */
    private fun exportFile(job: Job, name: String, target: Path, limit: Long): Long {
        val process = ProcessBuilder(exportCommand(job, name)).redirectError(ProcessBuilder.Redirect.DISCARD).start()
        var copied = 0L
        try {
            CompletableFuture.runAsync {
                TarArchiveInputStream(process.inputStream).use { tar ->
                    val entry = tar.nextEntry ?: return@use
                    check(entry.isFile && !entry.isSymbolicLink && !entry.isLink && entry.size in 0..limit) { "Invalid/oversized runner artifact" }
                    copied = copyEntry(tar, target, limit)
                    check(copied == entry.size)
                    check(tar.nextEntry == null) { "Expected one regular artifact" }
                }
            }.get(EXPORT_TIMEOUT_SECONDS, TimeUnit.SECONDS)
            if (!process.waitFor(EXPORT_EXIT_TIMEOUT_SECONDS, TimeUnit.SECONDS)) error("Artifact copy timed out")
        } finally {
            if (process.isAlive) process.destroyForcibly()
        }
        return copied
    }

    /**
     * Забирает записи кэша проб из замороженного тома: обычные файлы каталога `cache` с именами из белого списка,
     * каждый не больше лимита записи, всего не больше лимита группы; остальное (ссылки, каталоги, мусор) пропускается.
     */
    private fun exportCache(job: Job) {
        if (!props.probeCacheEnabled) return
        val target = Files.createDirectories(job.directory.resolve(ProbeCacheEntries.DIR))
        val process = ProcessBuilder(exportCommand(job, ProbeCacheEntries.DIR)).redirectError(ProcessBuilder.Redirect.DISCARD).start()
        try {
            CompletableFuture.runAsync {
                TarArchiveInputStream(process.inputStream).use { tar ->
                    var budget = props.probeCacheEntryMaxBytes * props.probeCacheGroupMaxEntries
                    repeat(MAX_CACHE_TAR_ENTRIES) {
                        val entry = tar.nextEntry ?: return@use
                        val name = entry.name.removePrefix("${ProbeCacheEntries.DIR}/")
                        val wanted = entry.isFile && !entry.isLink && ProbeCacheEntries.isEntryName(name) &&
                            entry.name == "${ProbeCacheEntries.DIR}/$name" && entry.size in 1..minOf(props.probeCacheEntryMaxBytes, budget)
                        if (wanted) budget -= copyEntry(tar, target.resolve(name), entry.size)
                    }
                }
            }.get(EXPORT_TIMEOUT_SECONDS, TimeUnit.SECONDS)
        } finally {
            if (process.isAlive) process.destroyForcibly()
        }
    }

    private fun copyEntry(tar: TarArchiveInputStream, target: Path, limit: Long): Long {
        var copied = 0L
        Files.newOutputStream(target).use { out ->
            val buffer = ByteArray(COPY_BUFFER_BYTES)
            while (true) {
                val read = tar.read(buffer)
                if (read < 0) break
                copied += read
                check(copied <= limit)
                out.write(buffer, 0, read)
            }
        }
        return copied
    }

    private fun exportCommand(job: Job, name: String) = listOf(
        "docker", "run", "--rm", "--platform", PLATFORM, "--name", "goat-export-${job.id}",
        "--label", "$WORKER_LABEL=${props.workerId}", "--network", "none", "--read-only", "--cap-drop", "ALL",
        "--security-opt", "no-new-privileges", "--pids-limit", "32", "--memory", "128m", "--cpus", "0.5",
        "--user", "$RUNNER_UID:$RUNNER_UID",
        "--mount", "type=volume,src=${volumes[job.id]},dst=/export,readonly,volume-nocopy",
        images[job.id]!!, "/bin/tar", "-C", "/export", "-cf", "-", "--", name,
    )

    private fun rayonThreads(): String = props.workerCpus.toInt().coerceAtLeast(1).toString()

    private fun isRegularFile(path: Path) = Files.isRegularFile(path, LinkOption.NOFOLLOW_LINKS)

    private companion object {
        const val WORKER_LABEL = "goat.worker"
        const val PLATFORM = "linux/amd64"
        const val RUNNER_UID = 1000
        const val WORK_DIR = "/work"
        const val IMAGE_BIN_DIR = "/usr/local/bin"
        const val INPUT_3MF = "input.3mf"
        const val INPUT_STL = "input.stl"
        const val COPY_BUFFER_BYTES = 64 * 1024
        const val TRANSFER_TIMEOUT_SECONDS = 30L
        const val EXPORT_TIMEOUT_SECONDS = 30L
        const val MAX_CACHE_TAR_ENTRIES = 256
        const val EXPORT_EXIT_TIMEOUT_SECONDS = 15L

        /** Файлы, которые выгружаются из тома раннера: результаты упаковки, расписания и калибровки (журнал пишет JobRunner). */
        val EXPORTED_FILES = listOf(
            JobArtifacts.PACKED_3MF,
            JobArtifacts.PACKED_REPORT,
            JobArtifacts.PACKED_STATE,
            "${JobArtifacts.PACK_WORK_DIR}/placements.json",
            JobArtifacts.SCHEDULE_CSV,
            JobArtifacts.PLATES_3MF,
            JobArtifacts.SCHEDULE_REPORT,
            JobArtifacts.CALIBRATION_A,
            JobArtifacts.CALIBRATION_B,
            JobArtifacts.CALIBRATION_INSTRUCTIONS,
            JobArtifacts.CALIBRATION_REPORT,
        )
    }
}
