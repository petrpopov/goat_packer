package com.goatpacker.web.service.server

import com.goatpacker.web.config.AppProperties
import com.goatpacker.web.config.PlatformProperties
import com.goatpacker.web.entity.PlatformEntity
import com.goatpacker.web.entity.QueueJobEntity
import com.goatpacker.web.entity.WorkerEntity
import com.goatpacker.web.entity.newId
import com.goatpacker.web.model.Job
import com.goatpacker.web.service.runner.NativeLauncher
import jakarta.annotation.PreDestroy
import org.slf4j.LoggerFactory
import org.slf4j.MDC
import org.springframework.boot.autoconfigure.condition.ConditionalOnProperty
import org.springframework.context.annotation.Profile
import org.springframework.scheduling.annotation.Scheduled
import org.springframework.stereotype.Component
import java.nio.channels.FileChannel
import java.nio.channels.FileLock
import java.nio.file.Files
import java.nio.file.LinkOption
import java.nio.file.StandardOpenOption
import java.time.Duration
import java.time.Instant
import java.time.OffsetDateTime
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import kotlin.math.floor

/**
 * Исполнитель очереди: регистрируется в БД, раз в пару секунд берёт задачи по числу свободных слотов и отдаёт их
 * пулу потоков ([WorkerJobProcessor]); heartbeat продлевает аренды и останавливает отменённые задачи.
 */
@Component
@Profile("server")
@ConditionalOnProperty(name = ["platform.worker-enabled"], havingValue = "true")
class QueueWorker(
    private val execution: JobExecutionService,
    private val nodes: WorkerNodeService,
    private val processor: WorkerJobProcessor,
    private val launcher: NativeLauncher,
    private val app: AppProperties,
    private val props: PlatformProperties,
) {
    private val log = LoggerFactory.getLogger(javaClass)
    private val bootId = newId()
    private val capacity = minOf(
        floor(props.executionCpuBudget / props.workerCpus).toInt(),
        (props.executionMemoryMib / props.workerMemoryMib).toInt(),
        PlatformEntity.MAX_PARALLELISM,
    ).coerceAtLeast(0)
    private val pool = Executors.newFixedThreadPool(capacity.coerceAtLeast(1))
    private val live = ConcurrentHashMap<String, Job>()
    private val fences = ConcurrentHashMap<String, Long>()
    private val cleanupPending = ConcurrentHashMap.newKeySet<String>()
    private val root = app.jobsRootPath.resolve(props.workerId)

    @Volatile private var closing = false

    @Volatile private var registered = false
    private var runtimeImages: Map<String, String> = emptyMap()
    private var host = ""
    private var lockChannel: FileChannel? = null
    private var workerLock: FileLock? = null

    /**
     * An OS file lock fences paused/duplicate JVMs on this host, not just DB heartbeats.
     * Root MUST be a persistent per-host scratch volume shared by controllers with this ID.
     */
    private fun register() {
        require(WORKER_ID_PATTERN.matches(props.workerId))
        Files.createDirectories(root)
        Files.deleteIfExists(root.resolve(READY_FILE))
        if (workerLock == null) acquireFileLock()
        runtimeImages = launcher.qualify()
        host = launcher.hostIdentity()
        check(nodes.beginRecovery(props.workerId, bootId, host))
        launcher.recoverNode() // confirm all old native processes are dead BEFORE releasing their DB slots
        nodes.confirmRecovered(props.workerId, bootId)
        removeLeftoverJobDirectories()
        registered = true
        log.info(
            "event=worker_ready workerId={} bootId={} capacity={} cpu={} memoryMiB={} runtimes={}",
            props.workerId, bootId, capacity, props.executionCpuBudget, props.executionMemoryMib, runtimeImages,
        )
    }

    private fun acquireFileLock() {
        lockChannel = FileChannel.open(root.resolve(LOCK_FILE), StandardOpenOption.CREATE, StandardOpenOption.WRITE)
        try {
            workerLock = lockChannel!!.tryLock() ?: error("Another controller holds this worker's filesystem lock")
        } catch (ex: Exception) {
            lockChannel?.close()
            lockChannel = null
            throw ex
        }
    }

    /** Каталоги задач прошлой загрузки уже не нужны: их задачи отданы на повтор. */
    private fun removeLeftoverJobDirectories() {
        Files.list(root).use { entries ->
            entries
                .filter { Files.isDirectory(it, LinkOption.NOFOLLOW_LINKS) && runCatching { UUID.fromString(it.fileName.toString()) }.isSuccess }
                .forEach { it.toFile().deleteRecursively() }
        }
    }

    @Scheduled(fixedDelay = DISPATCH_INTERVAL_MS, initialDelay = DISPATCH_INITIAL_DELAY_MS)
    fun dispatch() {
        if (closing) return
        if (!registered) register()
        val effective = if (hasScratchSpace()) capacity else 0
        val self = WorkerEntity(
            props.workerId, bootId, props.executionCpuBudget, props.executionMemoryMib * BYTES_PER_MIB,
            effective, live.size, Instant.now(), props.runnerMode.name.lowercase(), host,
        )
        if (!nodes.pulse(self)) {
            // Нас вытеснила более новая загрузка исполнителя: всё, что запущено, нужно остановить.
            closing = true
            live.values.forEach(launcher::terminate)
            return
        }
        Files.writeString(root.resolve(READY_FILE), Instant.now().toString())
        execution.recoverQueue()
        while (!closing && live.size < effective) {
            val record = execution.claim(props.workerId, bootId) ?: break
            val job = Job(record.id, record.kind, root.resolve(record.id).resolve(record.fence.toString()), OffsetDateTime.now())
            job.runtimeFence = record.fence
            job.runtimeTimeoutSeconds = Duration.between(Instant.now(), record.deadlineAt).seconds.coerceAtLeast(1)
            live[record.id] = job
            fences[record.id] = record.fence
            pool.submit { run(record, job) }
        }
    }

    @Scheduled(fixedDelay = HEARTBEAT_INTERVAL_MS, initialDelay = HEARTBEAT_INITIAL_DELAY_MS)
    fun heartbeat() {
        live.forEach { (id, job) ->
            if (id in cleanupPending) {
                if (launcher.terminate(job)) {
                    runCatching {
                        execution.retryStopped(id, fences[id] ?: UNKNOWN_FENCE, "Восстановление после сбоя исполнителя")
                        release(job)
                    }
                }
                return@forEach
            }
            val safe = runCatching { execution.extendLease(id, fences[id] ?: UNKNOWN_FENCE) }.getOrDefault(false)
            if (!safe) {
                job.cancel()
                launcher.terminate(job)
            }
        }
    }

    private fun run(record: QueueJobEntity, job: Job) {
        MDC.put("jobId", record.id)
        MDC.put("attemptFence", record.fence.toString())
        var retryReason: String? = null
        try {
            retryReason = processor.process(record, job, runtimeImages)
        } catch (ex: Exception) {
            log.error("event=job_failed jobId={} reason={}", record.id, ex.message)
            retryReason = "Временная ошибка инфраструктуры: ${ex.message?.take(MAX_FAILURE_REASON_CHARS)}"
        } finally {
            settle(record, job, retryReason)
            MDC.remove("jobId")
            MDC.remove("attemptFence")
        }
    }

    /**
     * Освобождает слот, только когда процесс и контейнер точно остановлены. Staged-объекты здесь никогда не
     * удаляются: commit в БД мог пройти при потерянном ответе, а метку GC снимает сама публикация.
     */
    private fun settle(record: QueueJobEntity, job: Job, retryReason: String?) {
        if (!launcher.terminate(job)) {
            cleanupPending.add(job.id)
            runCatching { execution.markRuntimeLost(record.id, record.fence, "Runtime cleanup pending; slot remains reserved") }
            return
        }
        val settled = runCatching {
            execution.retryStopped(record.id, record.fence, retryReason ?: STOPPED_BEFORE_COMPLETION)
        }.isSuccess
        if (settled) release(job) else cleanupPending.add(job.id)
    }

    private fun release(job: Job) {
        job.directory.toFile().deleteRecursively()
        live.remove(job.id)
        fences.remove(job.id)
        cleanupPending.remove(job.id)
    }

    private fun hasScratchSpace(): Boolean =
        Files.getFileStore(root).usableSpace > props.outputReservationBytes + props.maxUploadBytes + SCRATCH_HEADROOM_BYTES

    @PreDestroy
    fun stop() {
        closing = true
        Files.deleteIfExists(root.resolve(READY_FILE))
        live.values.forEach {
            it.cancel()
            launcher.terminate(it)
        }
        pool.shutdown()
        pool.awaitTermination(SHUTDOWN_WAIT_SECONDS, TimeUnit.SECONDS)
        pool.shutdownNow()
        if (live.values.all { launcher.terminate(it) }) {
            if (registered) runCatching { nodes.markStopped(props.workerId, bootId) }
            workerLock?.release()
            lockChannel?.close()
        }
    }

    private companion object {
        const val DISPATCH_INTERVAL_MS = 2000L
        const val DISPATCH_INITIAL_DELAY_MS = 3000L
        const val HEARTBEAT_INTERVAL_MS = 5000L
        const val HEARTBEAT_INITIAL_DELAY_MS = 5000L
        const val SHUTDOWN_WAIT_SECONDS = 20L
        const val MAX_FAILURE_REASON_CHARS = 500
        const val UNKNOWN_FENCE = -1L

        /** Исполнитель закончил работу над задачей, не опубликовав результат: слот освобождается, задача идёт на повтор. */
        const val STOPPED_BEFORE_COMPLETION = "Исполнитель остановлен до завершения задачи"
        const val BYTES_PER_MIB = 1024L * 1024

        /** Запас диска сверх входа и результата задачи: временные файлы слайсера. */
        const val SCRATCH_HEADROOM_BYTES = 512L * 1024 * 1024

        /** Файл-признак готовности исполнителя (для healthcheck контейнера). */
        const val READY_FILE = "ready"
        const val LOCK_FILE = "controller.lock"
        val WORKER_ID_PATTERN = Regex("[a-zA-Z0-9_-]{1,64}")
    }
}
