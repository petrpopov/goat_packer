package com.goatpacker.web.service.server

import com.fasterxml.jackson.databind.ObjectMapper
import com.fasterxml.jackson.databind.node.ObjectNode
import com.fasterxml.jackson.module.kotlin.readValue
import com.goatpacker.web.config.AppProperties
import com.goatpacker.web.config.PlatformProperties
import com.goatpacker.web.dto.JobRequest
import com.goatpacker.web.entity.ArtifactEntity
import com.goatpacker.web.entity.QueueJobEntity
import com.goatpacker.web.entity.newId
import com.goatpacker.web.model.CalibrationParams
import com.goatpacker.web.model.Job
import com.goatpacker.web.model.JobKind
import com.goatpacker.web.model.JobStatus
import com.goatpacker.web.model.PackDefaults
import com.goatpacker.web.model.PackParams
import com.goatpacker.web.model.ScheduleParams
import com.goatpacker.web.model.ScheduleSource
import com.goatpacker.web.repository.BlobRepository
import com.goatpacker.web.repository.ObjectGcRepository
import com.goatpacker.web.repository.ObjectStore
import com.goatpacker.web.security.toHex
import com.goatpacker.web.service.LocalSlicer
import com.goatpacker.web.service.StlInputs
import com.goatpacker.web.service.runner.JobArtifacts
import com.goatpacker.web.service.runner.JobRunner
import com.goatpacker.web.service.runner.NativeLauncher
import com.goatpacker.web.util.digest
import com.goatpacker.web.util.safeName
import org.slf4j.LoggerFactory
import org.springframework.boot.autoconfigure.condition.ConditionalOnProperty
import org.springframework.context.annotation.Profile
import org.springframework.stereotype.Component
import java.nio.file.Files
import java.nio.file.Path

/**
 * Выполнение одной взятой задачи на исполнителе: вход из хранилища → профиль → запуск бинарника → публикация
 * результатов. Остановку процесса и освобождение слота делает [QueueWorker].
 */
@Component
@Profile("server")
@ConditionalOnProperty(name = ["platform.worker-enabled"], havingValue = "true")
class WorkerJobProcessor(
    private val execution: JobExecutionService,
    private val blobs: BlobRepository,
    private val gc: ObjectGcRepository,
    private val store: ObjectStore,
    private val runner: JobRunner,
    private val launcher: NativeLauncher,
    private val app: AppProperties,
    private val props: PlatformProperties,
    private val mapper: ObjectMapper,
) {
    private val log = LoggerFactory.getLogger(javaClass)

    /**
     * Выполняет задачу [record] и публикует результат. Возвращает причину, по которой её нужно запустить заново
     * (остановку контейнера подтвердить не удалось), или null.
     */
    fun process(record: QueueJobEntity, job: Job, runtimeImages: Map<String, String>): String? {
        Files.createDirectories(job.directory)
        // STL (только упаковка) кладётся как input.stl: input.3mf из него собирает source_project перед
        // пакером. Расписание по упаковке STL берёт packed.3mf — это уже проект (имя у старых записей могло остаться .stl).
        val isStl = record.kind == JobKind.PACK && StlInputs.isStl(record.originalName)
        downloadInput(record, job, isStl)
        // Снимок профиля с постановки: каталожный json или разовый CustomProfile своего стола (slicer_kind orca).
        val profile = mapper.readTree(record.profileJson) as ObjectNode
        choosePlatformSlicer(record, job, profile, runtimeImages)
        Files.writeString(job.directory.resolve(JobArtifacts.PROFILE_FILE), mapper.writeValueAsString(profile))
        val request = mapper.readValue<JobRequest>(record.paramsJson)
        // Доводка «Нарезать стол»: состояние упаковки родителя рядом со входом.
        request.finishStateBlobId?.let { blobId ->
            store.get(blobs.findById(blobId).orElseThrow().objectKey, job.directory.resolve(JobArtifacts.FINISH_STATE))
        }

        job.onUpdate = WorkerProgressReporter(execution, record, mapper)::report
        if (!execution.extendLease(record.id, record.fence)) {
            job.cancel()
            return null
        }
        log.info(
            "event=job_started jobId={} fence={} kind={} image={} cpu={} memoryMiB={}",
            record.id, record.fence, record.kind, job.runtimeImage, props.workerCpus, props.workerMemoryMib,
        )
        run(record, job, request, if (isStl) sourceCommand(profile) else null)

        if (!launcher.terminate(job)) return CONTAINER_NOT_STOPPED
        if (job.cancelled) {
            execution.complete(record.id, record.fence, null, emptyList(), "Задача остановлена по лимиту или отменена")
            return null
        }
        publish(record, job, profile)
        return null
    }

    /** Скачивает вход в каталог джоба и сверяет контрольную сумму (у входов из артефактов её нет). */
    private fun downloadInput(record: QueueJobEntity, job: Job, isStl: Boolean) {
        val blob = blobs.findById(record.inputBlobId).orElseThrow()
        val input = job.directory.resolve(if (isStl) StlInputs.INPUT_STL else StlInputs.INPUT_3MF)
        store.get(blob.objectKey, input)
        if (SHA256_HEX.matches(blob.sha256)) check(input.digest("SHA-256").toHex() == blob.sha256) { "Input checksum mismatch" }
    }

    /** docker: слайсер образа (его id закрепляется за задачей); локально — слайсер из настроек. */
    private fun choosePlatformSlicer(record: QueueJobEntity, job: Job, profile: ObjectNode, runtimeImages: Map<String, String>) {
        if (props.localRunner) {
            LocalSlicer.localize(app, profile) // локальный раннер: слайсер из настроек, не путь образа
            return
        }
        val kind = profile.path("slicer_kind").asText()
        job.runtimeImage = execution.pinImage(record.id, record.fence, runtimeImages[kind] ?: error("Unsupported runtime"))
        profile.put("slicer_path", LocalSlicer.runnerSlicerPath(kind))
    }

    /** Экспорт STL — слайсером образа задачи и его же пресетами (в runner-bambu это Bambu Studio с BBL-деревом). */
    private fun sourceCommand(profile: ObjectNode): List<String> {
        if (!props.localRunner) {
            return StlInputs.command(SOURCE_PROJECT_BIN, StlInputs.RUNNER_PRESETS, profile.path("slicer_path").asText(), profile)
        }
        val (presets, slicer) = StlInputs.localTools(app, profile)
        return StlInputs.command(app.sourceProjectBinPath.toString(), presets, slicer, profile)
    }

    private fun run(record: QueueJobEntity, job: Job, request: JobRequest, sourceCommand: List<String>?) {
        val profilePath = Path.of(JobArtifacts.PROFILE_FILE)
        when (record.kind) {
            JobKind.PACK -> runner.runPackBlocking(
                job,
                PackParams(
                    profileId = record.profileId,
                    profilePath = profilePath,
                    clearance = request.clearance,
                    resolution = request.resolution,
                    attempts = request.attempts,
                    candidates = PackDefaults.CANDIDATES,
                    seed = PackDefaults.SEED,
                    edgeMargin = request.edgeMargin,
                    layout = request.layout,
                    nozzleClean = request.nozzleClean,
                    timeWindowHours = request.timeWindowHours,
                    amsCorrection = request.amsCorrection,
                    changeSeconds = request.changeSeconds,
                    timeMarginPct = request.timeMarginPct,
                    supportOverlapMm = request.supportOverlapMm,
                    sourceCommand = sourceCommand,
                    finalSlice = request.finalSlice,
                    finishState = request.finishStateBlobId?.let { Path.of(JobArtifacts.FINISH_STATE) },
                    slicer = request.slicer,
                ),
            )

            JobKind.CALIBRATION -> runner.runCalibrationBlocking(job, CalibrationParams(record.profileId, profilePath, request.variant))

            JobKind.SCHEDULE -> runner.runScheduleBlocking(
                job,
                ScheduleParams(
                    profileId = record.profileId,
                    profilePath = profilePath,
                    source = ScheduleSource.Uploaded,
                    dayStart = request.dayStart,
                    nightStart = request.nightStart,
                    horizonEnd = request.horizonEnd,
                    turnaroundMin = request.turnaroundMin,
                    prepMin = request.prepMin,
                    benchMode = request.benchMode,
                    amsCorrection = request.amsCorrection,
                    changeSeconds = request.changeSeconds,
                ),
            )
        }
    }

    /** Заливает результаты в хранилище под неизменяемыми ключами и фиксирует их вместе с итогом задачи. */
    private fun publish(record: QueueJobEntity, job: Job, profile: ObjectNode) {
        val uploaded = mutableListOf<ArtifactEntity>()
        var bytes = 0L
        for ((name, path) in JobArtifacts.listExisting(job.kind, job.directory)) {
            val size = Files.size(path)
            bytes += size
            check(bytes <= record.reservedBytes) { "Output exceeds reserved quota" }
            val key = "jobs/${record.id}/${record.fence}/${newId()}/$name"
            gc.stage(key, size)
            store.put(key, path)
            uploaded += ArtifactEntity(
                jobId = record.id,
                name = name,
                downloadName = downloadName(record, job, profile, name),
                objectKey = key,
                sizeBytes = size,
                debug = isDiagnostic(name),
            )
        }
        val succeeded = job.current.status == JobStatus.SUCCEEDED
        val committed = if (succeeded) {
            execution.complete(record.id, record.fence, resultJson(job), uploaded, null)
        } else {
            execution.completeFailed(record.id, record.fence, uploaded.filter { it.debug }, job.current.error ?: "Ошибка обработки")
        }
        log.info("event=job_finished jobId={} state={} published={}", record.id, job.current.status, committed)
    }

    private fun resultJson(job: Job): String = mapper.writeValueAsString(
        when (job.kind) {
            JobKind.PACK -> job.packResult
            JobKind.CALIBRATION -> job.calibrationResult
            JobKind.SCHEDULE -> job.scheduleResult
        },
    )

    /** Диагностика (отчёты и журнал) живёт недолго и скрыта по умолчанию; calibration.json — результат, а не диагностика. */
    private fun isDiagnostic(name: String) = (name.endsWith(".json") && name != "calibration.json") || name.endsWith(".txt")

    /** Имя для пользователя: `<файл>_<принтер>_<число объектов>_packed|schedule.3mf`; прочие файлы и калибровка — как есть. */
    private fun downloadName(record: QueueJobEntity, job: Job, profile: ObjectNode, name: String): String {
        // Калибровочные файлы сохраняют свои имена: instructions.md ссылается на calib_A/calib_B.
        if (record.kind == JobKind.CALIBRATION || !name.endsWith(".3mf")) return name
        val base = safeName(record.originalName).replace(MODEL_EXTENSION, "").take(MAX_BASE_NAME_CHARS)
        val slug = profile.path("download_slug").asText(record.profileId).replace(UNSAFE_SLUG_CHARS, "_").take(MAX_SLUG_CHARS)
        val count = job.packResult?.objectCount ?: job.scheduleResult?.totalObjects ?: 0
        val role = if (record.kind == JobKind.PACK) "packed" else "schedule"
        return "${base}_${slug}_${count}_$role.3mf"
    }

    companion object {
        /** Причина повторного запуска, когда контейнер не удалось остановить. */
        const val CONTAINER_NOT_STOPPED = "Остановка контейнера не подтверждена"

        private const val SOURCE_PROJECT_BIN = "source_project"
        private const val MAX_BASE_NAME_CHARS = 80
        private const val MAX_SLUG_CHARS = 40
        private val SHA256_HEX = Regex("[0-9a-f]{64}")
        private val MODEL_EXTENSION = Regex("(?i)\\.(3mf|stl)$")
        private val UNSAFE_SLUG_CHARS = Regex("[^a-zA-Z0-9_-]")
    }
}
