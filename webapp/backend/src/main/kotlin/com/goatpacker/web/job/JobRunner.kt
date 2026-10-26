package com.goatpacker.web.job

import com.fasterxml.jackson.databind.JsonNode
import com.fasterxml.jackson.databind.ObjectMapper
import com.goatpacker.web.config.AppProperties
import com.goatpacker.web.service.ClockFormat
import com.goatpacker.web.service.ProgressTracker
import com.goatpacker.web.service.XlsxWriter
import org.slf4j.LoggerFactory
import org.springframework.scheduling.annotation.Async
import org.springframework.stereotype.Component
import java.nio.charset.StandardCharsets
import java.nio.file.Files
import java.nio.file.Path
import java.nio.file.StandardOpenOption
import java.time.OffsetDateTime

/** Параметры pack-джоба — транслируются в CLI-аргументы colored_3mf_packer один в один. */
data class PackParams(
    val profileId: String,
    /** Путь к json профиля, разрешённый ProfileService (id профиля ≠ имя файла). */
    val profilePath: Path,
    val clearance: Double,
    val resolution: Double,
    val attempts: Long,
    val candidates: Long,
    val seed: Long,
    /** Override отступа от края стола; null = дефолт профиля. */
    val edgeMargin: Double?,
    /** search = детерминированный поиск, grid = только детерминированная сетка (быстро). */
    val layout: String,
    val nozzleClean: String,
    /** Окно печати, ч → --time-window-hours; null = без окна. */
    val timeWindowHours: Double? = null,
    /** --ams-correction; false = флаг не передаётся. */
    val amsCorrection: Boolean = false,
    /** --change-seconds (передаётся только вместе с [amsCorrection]); null = значение профиля. */
    val changeSeconds: Double? = null,
    /** --time-margin-pct; null = дефолт CLI (10 %). */
    val timeMarginPct: Double? = null,
)

/** Источник 3MF для schedule-джоба. */
sealed interface ScheduleSource {
    /** Собственный input.3mf джоба (каталог джоба — рабочий каталог процесса). */
    data object Uploaded : ScheduleSource

    /** Готовый упакованный файл из успешного pack-джоба. */
    data class FromPackJob(val packedPath: Path, val profileId: String, val job: Job) : ScheduleSource
}

/** Параметры schedule-джоба — транслируются в CLI-аргументы daily_scheduler один в один. */
data class ScheduleParams(
    val profileId: String,
    /** Путь к json профиля, разрешённый ProfileService (id профиля ≠ имя файла). */
    val profilePath: Path,
    val source: ScheduleSource,
    val dayStart: String,
    val nightStart: String,
    val horizonEnd: String,
    val turnaroundMin: Int,
    val prepMin: Int,
    val benchMode: String,
    /** Глобальный --ams-correction. */
    val amsCorrection: Boolean = false,
    /** Ключ change-s= в спеке --printer (только вместе с [amsCorrection]); null = значение профиля. */
    val changeSeconds: Double? = null,
)

/**
 * Запуск фоновых джоб: ProcessBuilder (stdout+stderr слиты), построчная запись в log.txt и
 * синхронный разбор прогресса. Таймаута процесса нет — слайсинг легитимно долгий; отмена —
 * через [JobProcessKiller] при DELETE.
 */
@Component
class JobRunner(
    private val properties: AppProperties,
    private val objectMapper: ObjectMapper,
    private val xlsxWriter: XlsxWriter,
    private val nativeLauncher: org.springframework.beans.factory.ObjectProvider<com.goatpacker.web.server.NativeLauncher>? = null,
) {
    companion object {
        private val log = LoggerFactory.getLogger(JobRunner::class.java)

        /** Сколько последних строк лога идёт в error при FAILED. */
        private const val ERROR_TAIL_LINES = 15

        private val PACK_REQUIRED = listOf("packed.3mf")
        private val SCHEDULE_REQUIRED = listOf("schedule.csv", "plates.3mf")
    }

    @Async("jobExecutor")
    fun runPack(job: Job, params: PackParams) = runPackBlocking(job, params)

    fun runPackBlocking(job: Job, params: PackParams) {
        val command = buildList {
            add(properties.packerBinPath.toString())
            add("--source")
            add("input.3mf")
            add("--profile")
            add(params.profilePath.toString())
            add("--output")
            add("packed.3mf")
            add("--work-dir")
            add("work")
            add("--clearance")
            add(params.clearance.toString())
            add("--resolution")
            add(params.resolution.toString())
            add("--attempts")
            add(params.attempts.toString())
            add("--candidates")
            add(params.candidates.toString())
            add("--seed")
            add(params.seed.toString())
            params.edgeMargin?.let {
                add("--edge-margin")
                add(it.toString())
            }
            add("--layout")
            add(params.layout)
            if (params.nozzleClean.isNotBlank() && !params.nozzleClean.equals("off", ignoreCase = true)) {
                add("--nozzle-clean")
                add(params.nozzleClean)
            }
            params.timeWindowHours?.let {
                add("--time-window-hours")
                add(it.toString())
            }
            // --change-seconds сам включает коррекцию в CLI, поэтому без галочки его не передаём:
            // иначе значение, оставшееся в форме, молча включило бы коррекцию.
            if (params.amsCorrection) {
                add("--ams-correction")
                params.changeSeconds?.let {
                    add("--change-seconds")
                    add(it.toString())
                }
            }
            params.timeMarginPct?.let {
                add("--time-margin-pct")
                add(it.toString())
            }
        }

        execute(job, command) { exitCode -> finishPack(job, params, exitCode) }
    }

    @Async("jobExecutor")
    fun runSchedule(job: Job, params: ScheduleParams) = runScheduleBlocking(job, params)

    fun runScheduleBlocking(job: Job, params: ScheduleParams) {
        val source = "input.3mf"
        val command = buildList {
            add(properties.schedulerBinPath.toString())
            // ВАЖНО: аргумент после --printer — одна строка с запятыми.
            add("--printer")
            add(
                "profile=${params.profilePath}," +
                    "source=$source," +
                    "prep-min=${params.prepMin}," +
                    // change-s= сам включает коррекцию — только вместе с --ams-correction.
                    (params.changeSeconds?.takeIf { params.amsCorrection }?.let { "change-s=$it," } ?: "") +
                    "plates-3mf=plates.3mf",
            )
            if (params.amsCorrection) add("--ams-correction")
            add("--output-schedule")
            add("schedule.csv")
            add("--output-report")
            add("report.json")
            add("--turnaround-min")
            add(params.turnaroundMin.toString())
            add("--day-start")
            add(params.dayStart)
            add("--night-start")
            add(params.nightStart)
            add("--horizon-end")
            add(params.horizonEnd)
            if (params.benchMode.equals("full", ignoreCase = true)) {
                add("--bench-mode")
                add("full")
            }
        }

        execute(job, command) { exitCode -> finishSchedule(job, exitCode) }
    }

    private fun execute(job: Job, command: List<String>, finish: (Int) -> Unit) {
        log.info("Джоба {} ({}) запущена: {}", job.id, job.kind, command.joinToString(" "))
        if (job.cancelled) return
        val tracker = ProgressTracker(job.kind)

        try {
            val process = synchronized(job) {
                if (job.cancelled) return
                (nativeLauncher?.getIfAvailable()?.start(job, command) ?: ProcessBuilder(command)
                    .redirectErrorStream(true)
                    .directory(job.directory.toFile())
                    .start()).also {
                        job.process = it
                        job.update { snap -> snap.copy(status = JobStatus.RUNNING) }
                    }
            }

            val logFile = job.directory.resolve(JobArtifacts.LOG_FILE)
            java.io.BufferedWriter(
                java.io.OutputStreamWriter(
                    Files.newOutputStream(logFile, StandardOpenOption.CREATE, StandardOpenOption.APPEND),
                    StandardCharsets.UTF_8,
                ),
            ).use { writer ->
                var written = 0L
                com.goatpacker.web.job.BoundedLines.read(process.inputStream) { line ->
                    if (written < 10L * 1024 * 1024) {
                        writer.write(line);writer.newLine();writer.flush();written += line.toByteArray(StandardCharsets.UTF_8).size + 1
                    }
                    val event = tracker.onLine(line)
                    if (line.contains("event=") || line.contains("\"event\"")) log.info("jobId={} component=cli message={}", job.id, line.take(8192))
                    job.update {
                        it.copy(
                            phase = event.phase,
                            progressPercent = event.progressPercent,
                            phaseMessage = event.phaseMessage,
                        )
                    }
                }
            }

            val exitCode = process.waitFor()
            log.info("Джоба {} завершилась с кодом {}", job.id, exitCode)
            if (!job.cancelled) {
                nativeLauncher?.getIfAvailable()?.collect(job)
                finish(exitCode)
            }
        } catch (ex: Exception) {
            log.error("Джоба {} упала: {}", job.id, ex.message, ex)
            if (!job.cancelled) fail(job, "Процесс завершился аварийно: ${ex.message ?: ex::class.java.simpleName}")
        } finally {
            nativeLauncher?.getIfAvailable()?.stop(job)
            job.process?.takeIf { it.isAlive }?.let { JobProcessKiller.kill(it) }
            synchronized(job) { job.process = null }
            if (job.cancelled) job.directory.toFile().deleteRecursively()
        }
    }

    private fun finishPack(job: Job, params: PackParams, exitCode: Int) {
        if (!succeeded(job, exitCode, PACK_REQUIRED)) {
            return
        }

        val report = job.directory.resolve("packed.verification.json")
            .takeIf { Files.exists(it) }
            ?.let { objectMapper.readTree(it.toFile()) }
        val objectCount = report
            ?.get("object_count")
            ?.asInt(-1)
            ?: -1
        if (objectCount < 0) {
            fail(job, "Не удалось разобрать verification.json: поле object_count отсутствует")
            return
        }

        job.packResult = PackJobResult(
            objectCount = objectCount,
            profileId = params.profileId,
            clearanceMm = params.clearance,
            time = report?.get("time")?.let(::parsePackTime),
        )
        succeed(job, JobPhase.CLEANUP)
    }

    /**
     * Блок `time` отчёта упаковщика → [PackTimeResult]. null/битый блок = время не измерено
     * (стол при этом валиден), поэтому ошибкой джобы это не считается. Полный стол — наибольший
     * count среди замеров: с окном это второй замер (после пробы на 1 объект), без окна — сам стол.
     */
    private fun parsePackTime(node: JsonNode): PackTimeResult? {
        if (!node.isObject) return null
        val corrected = node.get("corrected_s")?.takeIf { it.isNumber }?.asDouble() ?: return null
        val chosen = node.get("chosen_count")?.takeIf { it.canConvertToInt() }?.asInt() ?: return null
        val fullCount = node.path("samples").mapNotNull { sample -> sample.get("count")?.takeIf { it.isInt }?.asInt() }
            .maxOrNull() ?: chosen
        return PackTimeResult(
            windowHours = node.get("window_hours")?.takeIf { it.isNumber }?.asDouble(),
            marginPct = node.path("margin_pct").asDouble(0.0),
            realChangeSeconds = node.get("real_change_s")?.takeIf { it.isNumber }?.asDouble(),
            chosenCount = chosen,
            fullCount = maxOf(fullCount, chosen),
            correctedSeconds = corrected,
        )
    }

    private fun finishSchedule(job: Job, exitCode: Int) {
        if (!succeeded(job, exitCode, SCHEDULE_REQUIRED)) {
            return
        }

        val result = parseScheduleReport(job)
        if (result == null) {
            fail(job, "Не удалось разобрать report.json: ожидается printers[0].variable_batch_schedule")
            return
        }

        // XLSX пишется после успеха из тех же данных, что и schedule.csv.
        try {
            xlsxWriter.write(job.directory.resolve("schedule.xlsx"), result.printer, result.jobs)
        } catch (ex: Exception) {
            // Обязательные артефакты (schedule.csv, plates.3mf) на месте — джоба успешна,
            // просто xlsx-артефакт не появится.
            log.error("Не удалось записать schedule.xlsx для джобы {}: {}", job.id, ex.message, ex)
        }

        job.scheduleResult = result
        succeed(job, JobPhase.WRITING)
    }

    /** Общая проверка успеха: exit 0 + обязательные артефакты на месте. При провале — FAILED. */
    private fun succeeded(job: Job, exitCode: Int, required: List<String>): Boolean {
        val missing = required.filter { !Files.isRegularFile(job.directory.resolve(it)) || Files.size(job.directory.resolve(it)) == 0L }
        if (exitCode != 0 || missing.isNotEmpty()) {
            val reason = buildString {
                append("Процесс завершился с кодом $exitCode")
                if (missing.isNotEmpty()) {
                    append("; отсутствуют артефакты: ")
                    append(missing.joinToString(", "))
                }
            }
            fail(job, reason)
            return false
        }

        return true
    }

    private fun succeed(job: Job, finalPhase: JobPhase) {
        job.update {
            it.copy(
                status = JobStatus.SUCCEEDED,
                phase = finalPhase,
                progressPercent = 100,
                error = null,
                finishedAt = OffsetDateTime.now(),
            )
        }
    }

    private fun fail(job: Job, reason: String) {
        log.warn("Джоба {} не удалась: {}", job.id, reason)
        job.update {
            it.copy(
                status = JobStatus.FAILED,
                error = friendlyError(job, reason) ?: reason,
                finishedAt = OffsetDateTime.now(),
            )
        }
    }

    /**
     * Хвост лога с переведённым заголовком: известные ошибки бинарников получают
     * человекопонятное «что случилось и что делать», сырой хвост остаётся под разделителем
     * (фронт показывает его свёрнутым). Неопознанный хвост возвращается как есть.
     */
    private fun friendlyError(job: Job, reason: String): String? {
        val tail = logTail(job.directory, ERROR_TAIL_LINES) ?: return null
        val headline = CliErrorTranslator.translate(tail) ?: reason
        return headline + CliErrorTranslator.DETAILS_SEPARATOR + tail
    }

    /** Хвост лога: последние [lines] строк log.txt. */
    private fun logTail(jobDir: Path, lines: Int): String? {
        val logFile = jobDir.resolve(JobArtifacts.LOG_FILE)
        if (!Files.exists(logFile)) {
            return null
        }

        return try {
            val tail = java.util.ArrayDeque<String>(lines)
            Files.newBufferedReader(logFile, StandardCharsets.UTF_8).useLines { stream ->
                stream.forEach { line ->
                    if (tail.size == lines) tail.removeFirst()
                    tail.addLast(line.takeLast(4096))
                }
            }
            tail.joinToString("\n")
        } catch (_: Exception) { null }
    }

    /** report.json → printers[0].variable_batch_schedule → [ScheduleJobResult] (plates считаются тут). */
    private fun parseScheduleReport(job: Job): ScheduleJobResult? {
        val reportPath = job.directory.resolve("report.json")
        if (!Files.exists(reportPath)) {
            return null
        }

        val root = try {
            objectMapper.readTree(reportPath.toFile())
        } catch (ex: Exception) {
            log.error("report.json джобы {} не читается: {}", job.id, ex.message)
            return null
        }
        val printerAnalysis = root.get("printers")?.get(0) ?: return null
        val schedule = printerAnalysis.get("variable_batch_schedule") ?: return null
        val jobsNode = schedule.get("jobs")?.takeIf { it.isArray } ?: return null

        val jobs = jobsNode.map { node ->
            ScheduleJobRecord(
                sequence = node.path("sequence").asInt(),
                batchSize = node.path("batch_size").asInt(),
                startSeconds = node.path("start_seconds").asLong(),
                endSeconds = node.path("end_seconds").asLong(),
                slicerSeconds = node.path("slicer_seconds").asLong(),
                preparationSeconds = node.path("preparation_seconds").asLong(),
                filamentGrams = node.path("filament_grams").asDouble(),
                cumulativeObjects = node.path("cumulative_objects").asInt(),
                overnight = node.path("overnight").asBoolean(),
            )
        }
        if (jobs.isEmpty()) {
            return null
        }

        // Пластины: уникальные batch_size в порядке появления, подпись — по первой работе батча.
        val plates = linkedMapOf<Int, ScheduleJobRecord>()
        for (jobRecord in jobs) {
            plates.putIfAbsent(jobRecord.batchSize, jobRecord)
        }

        return ScheduleJobResult(
            printer = printerAnalysis.path("printer").asText(""),
            totalObjects = schedule.path("total_objects").asInt(0),
            totalFilamentGrams = schedule.path("total_filament_grams").asDouble(0.0),
            completionClock = ClockFormat.format(schedule.path("completion_seconds").asLong(0)),
            jobs = jobs,
            plates = plates.map { (batchSize, firstJob) ->
                PlateRecord(
                    label = "$batchSize pcs @ ${ClockFormat.format(firstJob.startSeconds)}",
                    objectCount = batchSize,
                )
            },
        )
    }
}
