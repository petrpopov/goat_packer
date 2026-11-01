package com.goatpacker.web.service.runner

import com.goatpacker.web.config.AppProperties
import com.goatpacker.web.config.JOB_EXECUTOR
import com.goatpacker.web.model.CalibrationParams
import com.goatpacker.web.model.Job
import com.goatpacker.web.model.JobPhase
import com.goatpacker.web.model.JobStatus
import com.goatpacker.web.model.PackParams
import com.goatpacker.web.model.ScheduleParams
import com.goatpacker.web.service.StlInputs
import com.goatpacker.web.util.BoundedLines
import com.goatpacker.web.util.ProcessKiller
import com.goatpacker.web.util.ShellChain
import org.slf4j.LoggerFactory
import org.springframework.beans.factory.ObjectProvider
import org.springframework.scheduling.annotation.Async
import org.springframework.stereotype.Component
import java.nio.file.Files
import java.time.OffsetDateTime

/**
 * Запуск фоновых джоб: ProcessBuilder (stdout+stderr слиты), построчная запись в log.txt и
 * синхронный разбор прогресса. Таймаута процесса нет — слайсинг легитимно долгий; отмена —
 * через [ProcessKiller] при DELETE. В серверном режиме процесс запускает [NativeLauncher] (контейнер).
 */
@Component
class JobRunner(
    private val properties: AppProperties,
    private val reports: JobReportParser,
    private val xlsxWriter: XlsxWriter,
    private val nativeLauncher: ObjectProvider<NativeLauncher>? = null,
) {
    private val log = LoggerFactory.getLogger(javaClass)

    @Async(JOB_EXECUTOR)
    fun runPack(job: Job, params: PackParams) = runPackBlocking(job, params)

    fun runPackBlocking(job: Job, params: PackParams) {
        if (params.sourceCommand != null) {
            // Конвертация молчит до конца: этап виден сразу, процент — в начале полосы пробы.
            job.update {
                it.copy(phase = JobPhase.PROBE, progressPercent = PackStage.PROBE_SLICE.bandStart, phaseMessage = StlInputs.CONVERSION_MESSAGE)
            }
        }
        val command = CliCommands.pack(properties.packerBinPath.toString(), params)
        execute(job, command, params.sourceCommand) { exitCode -> finishPack(job, params, exitCode) }
    }

    @Async(JOB_EXECUTOR)
    fun runSchedule(job: Job, params: ScheduleParams) = runScheduleBlocking(job, params)

    fun runScheduleBlocking(job: Job, params: ScheduleParams) {
        val command = CliCommands.schedule(properties.schedulerBinPath.toString(), params)
        execute(job, command) { exitCode -> finishSchedule(job, exitCode) }
    }

    @Async(JOB_EXECUTOR)
    fun runCalibration(job: Job, params: CalibrationParams) = runCalibrationBlocking(job, params)

    /**
     * ams_calibration генерирует два проекта и нарезает оба (1–2 мин) молча, без строк прогресса,
     * поэтому фаза «нарезка» выставляется сразу при старте.
     */
    fun runCalibrationBlocking(job: Job, params: CalibrationParams) {
        job.update {
            it.copy(
                phase = JobPhase.SLICING,
                progressPercent = ProgressTracker.CALIBRATION_START_PERCENT,
                phaseMessage = ProgressTracker.CALIBRATION_MESSAGE,
            )
        }
        val command = CliCommands.calibration(properties.calibrationBinPath.toString(), params)
        execute(job, command) { exitCode -> finishCalibration(job, params, exitCode) }
    }

    /** [prelude] — подготовительная команда (source_project), см. [ShellChain]. */
    private fun execute(job: Job, command: List<String>, prelude: List<String>? = null, finish: (Int) -> Unit) {
        log.info("Джоба {} ({}) запущена: {}", job.id, job.kind, (prelude?.let { it.joinToString(" ") + " && " }.orEmpty()) + command.joinToString(" "))
        if (job.cancelled) return
        val tracker = ProgressTracker(job.kind, initialMessage = prelude?.let { StlInputs.CONVERSION_MESSAGE })
        val launcher = nativeLauncher?.getIfAvailable()

        try {
            val process = startProcess(job, command, prelude, launcher) ?: return
            JobLogWriter(job.directory.resolve(JobArtifacts.LOG_FILE)).use { logFile ->
                BoundedLines.read(process.inputStream) { line ->
                    logFile.append(line)
                    logCliLine(job, line)
                    publish(job, tracker.onLine(line))
                }
            }

            val exitCode = process.waitFor()
            log.info("Джоба {} завершилась с кодом {}", job.id, exitCode)
            if (!job.cancelled) {
                launcher?.collect(job)
                finish(exitCode)
            }
        } catch (ex: Exception) {
            log.error("Джоба {} упала: {}", job.id, ex.message, ex)
            if (!job.cancelled) fail(job, "Процесс завершился аварийно: ${ex.message ?: ex::class.java.simpleName}")
        } finally {
            launcher?.stop(job)
            job.process?.takeIf { it.isAlive }?.let { ProcessKiller.kill(it) }
            synchronized(job) { job.process = null }
            if (job.cancelled) job.directory.toFile().deleteRecursively()
        }
    }

    /** Запускает процесс и отмечает джоб выполняющимся; null — джоб отменили до запуска. */
    private fun startProcess(job: Job, command: List<String>, prelude: List<String>?, launcher: NativeLauncher?): Process? =
        synchronized(job) {
            if (job.cancelled) return null
            val process = launcher?.start(job, command, prelude)
                ?: ProcessBuilder(ShellChain.command(prelude, command))
                    .redirectErrorStream(true)
                    .directory(job.directory.toFile())
                    // JSON-события (stage, layout) — тот же протокол, что в серверном/docker-режиме.
                    .also { it.environment()[LOG_FORMAT_ENV] = "json" }
                    .start()
            job.process = process
            job.update { it.copy(status = JobStatus.RUNNING) }
            process
        }

    private fun logCliLine(job: Job, line: String) {
        when {
            // layout — крупная строка: в лог только факт, иначе он забивается контурами.
            JobLogs.isLayoutEvent(line) -> log.info("jobId={} component=cli event=layout bytes={}", job.id, line.length)
            // slicer_output/slicer_progress сыплются каждые пару секунд — на DEBUG.
            JobLogs.isSlicerEvent(line) -> log.debug("jobId={} component=cli message={}", job.id, line.take(MAX_LOGGED_CHARS))
            line.contains("event=") || line.contains("\"event\"") -> log.info("jobId={} component=cli message={}", job.id, line.take(MAX_LOGGED_CHARS))
        }
    }

    private fun publish(job: Job, event: ProgressEvent) {
        job.update {
            it.copy(
                phase = event.phase,
                progressPercent = event.progressPercent,
                phaseMessage = event.phaseMessage.ifBlank { it.phaseMessage },
                stage = event.stage,
                layout = event.layout ?: it.layout,
            )
        }
    }

    private fun finishPack(job: Job, params: PackParams, exitCode: Int) {
        if (!succeeded(job, exitCode, JobArtifacts.REQUIRED_PACK)) return
        val result = reports.parsePack(job.directory, params)
        if (result == null) {
            fail(job, "Не удалось разобрать verification.json: поле object_count отсутствует")
            return
        }
        job.packResult = result
        succeed(job, JobPhase.CLEANUP)
    }

    private fun finishCalibration(job: Job, params: CalibrationParams, exitCode: Int) {
        if (!succeeded(job, exitCode, JobArtifacts.REQUIRED_CALIBRATION)) return
        job.calibrationResult = try {
            reports.parseCalibration(job.directory, params)
        } catch (ex: Exception) {
            fail(job, "Не удалось разобрать calibration.json: ${ex.message}")
            return
        }
        succeed(job, JobPhase.WRITING)
    }

    private fun finishSchedule(job: Job, exitCode: Int) {
        if (!succeeded(job, exitCode, JobArtifacts.REQUIRED_SCHEDULE)) return
        val result = reports.parseSchedule(job.directory, job.id)
        if (result == null) {
            fail(job, "Не удалось разобрать report.json: ожидается printers[0].variable_batch_schedule")
            return
        }
        // XLSX пишется после успеха из тех же данных, что и schedule.csv.
        try {
            xlsxWriter.write(job.directory.resolve(JobArtifacts.SCHEDULE_XLSX), result.printer, result.jobs)
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
        val missing = required.filter {
            val file = job.directory.resolve(it)
            !Files.isRegularFile(file) || Files.size(file) == 0L
        }
        if (exitCode == 0 && missing.isEmpty()) return true

        val reason = buildString {
            append("Процесс завершился с кодом $exitCode")
            if (missing.isNotEmpty()) append("; отсутствуют артефакты: ${missing.joinToString(", ")}")
        }
        fail(job, reason)
        return false
    }

    private fun succeed(job: Job, finalPhase: JobPhase) {
        job.update {
            it.copy(
                status = JobStatus.SUCCEEDED,
                phase = finalPhase,
                progressPercent = FINISHED_PERCENT,
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
                error = JobLogs.friendlyError(job.directory, reason) ?: reason,
                finishedAt = OffsetDateTime.now(),
            )
        }
    }

    private companion object {
        const val FINISHED_PERCENT = 100
        const val MAX_LOGGED_CHARS = 8192
        const val LOG_FORMAT_ENV = "GOAT_LOG_FORMAT"
    }
}
