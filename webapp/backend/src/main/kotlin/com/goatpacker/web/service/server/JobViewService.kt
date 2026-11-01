package com.goatpacker.web.service.server

import com.fasterxml.jackson.databind.JsonNode
import com.fasterxml.jackson.databind.ObjectMapper
import com.fasterxml.jackson.databind.node.ObjectNode
import com.goatpacker.web.dto.JobRequest
import com.goatpacker.web.dto.JobStageDto
import com.goatpacker.web.dto.ProfileDto
import com.goatpacker.web.dto.QueueArtifactDto
import com.goatpacker.web.dto.QueueCheckDto
import com.goatpacker.web.dto.QueueJobDetailDto
import com.goatpacker.web.dto.QueueJobPageDto
import com.goatpacker.web.dto.QueueJobSummaryDto
import com.goatpacker.web.dto.WaitEstimate
import com.goatpacker.web.dto.toPackParamsDto
import com.goatpacker.web.entity.PlatformEntity
import com.goatpacker.web.entity.QueueJobEntity
import com.goatpacker.web.entity.QueueStatus
import com.goatpacker.web.model.JobKind
import com.goatpacker.web.model.JobStage
import com.goatpacker.web.repository.ArtifactRepository
import com.goatpacker.web.repository.PlatformRepository
import com.goatpacker.web.repository.QueueJobRepository
import com.goatpacker.web.repository.QueueJobSqlRepository
import com.goatpacker.web.repository.WorkerRepository
import com.goatpacker.web.service.CustomProfile
import com.goatpacker.web.service.ProfileService
import com.goatpacker.web.service.runner.CliErrorTranslator
import com.goatpacker.web.util.ClockFormat
import org.springframework.context.annotation.Profile
import org.springframework.data.domain.PageRequest
import org.springframework.data.domain.Sort
import org.springframework.stereotype.Service

/** Статусы задач для API: список, статус одной задачи, проверка нарезкой внутри неё и оценка ожидания. */
@Service
@Profile("server")
class JobViewService(
    private val jobs: QueueJobRepository,
    private val artifacts: ArtifactRepository,
    private val platform: PlatformRepository,
    private val workers: WorkerRepository,
    private val queueSql: QueueJobSqlRepository,
    private val access: JobAccessService,
    private val profiles: ProfileService,
    private val mapper: ObjectMapper,
) {
    fun list(viewerId: String, page: Int): QueueJobPageDto {
        Paging.requirePage(page)
        val pageable = PageRequest.of(page, JOBS_PAGE_SIZE, Sort.by("createdAt").descending())
        val rows = jobs.findAllByOwnerIdAndParentJobIdIsNull(viewerId, pageable)
        return QueueJobPageDto(rows.content.map { summary(it, viewerId, debug = false, withStage = false) }, rows.totalElements)
    }

    /**
     * Полный статус задачи: сводка плюс раскладка, профиль и настройки упаковки. [debug] открывает диагностику и
     * сырую ошибку. Доступ: владелец, выданный доступ или [token] ссылки.
     */
    fun get(viewerId: String, jobId: String, token: String?, debug: Boolean): QueueJobDetailDto {
        val job = access.findReadable(jobId, viewerId, token)
        // Профиль и настройки — только в статусе одной задачи: список лёгкий.
        val request = if (job.kind == JobKind.PACK) runCatching { mapper.readValue(job.paramsJson, JobRequest::class.java) }.getOrNull() else null
        // Раскладка проверки (полная нарезка могла сдвинуть объекты) важнее раскладки упаковки, как только она есть.
        val layout = (if (job.kind == JobKind.PACK) access.checkOf(job)?.layoutJson else null) ?: job.layoutJson
        return QueueJobDetailDto(
            summary = summary(job, viewerId, debug, withStage = true),
            layout = layout?.let { runCatching { mapper.readTree(it) }.getOrNull() },
            profile = request?.let { profileOf(job, it) },
            params = request?.toPackParamsDto(),
        )
    }

    private fun summary(job: QueueJobEntity, viewerId: String, debug: Boolean, withStage: Boolean): QueueJobSummaryDto {
        val wait = waitEstimate(job)
        val result = job.resultJson?.let { mapper.readTree(it) }
        if (job.kind == JobKind.SCHEDULE) addScheduleClocks(result)
        return QueueJobSummaryDto(
            canManage = job.ownerId == viewerId,
            jobId = job.id,
            kind = job.kind.jsonValue(),
            status = job.status,
            phase = job.phase,
            progressPercent = job.progress,
            phaseMessage = job.message,
            error = errorOf(job, debug),
            createdAt = job.createdAt,
            finishedAt = job.finishedAt,
            expiresAt = job.expiresAt,
            originalName = job.originalName,
            queuePosition = wait.position,
            estimatedWaitSeconds = wait.seconds,
            estimatedWaitFromSeconds = wait.fromSeconds,
            estimatedWaitToSeconds = wait.toSeconds,
            estimateReason = wait.reason,
            artifacts = artifactsOf(job, ownerId = job.id, debug = debug),
            pack = result.takeIf { job.kind == JobKind.PACK },
            schedule = result.takeIf { job.kind == JobKind.SCHEDULE },
            calibration = result.takeIf { job.kind == JobKind.CALIBRATION },
            stage = stageOf(job),
            // Проверка нарезкой — часть этой задачи (у списка без стадии: она там не нужна).
            check = if (job.kind == JobKind.PACK) checkOf(job, debug, withStage) else null,
        )
    }

    /** Проверка нарезкой внутри задачи (null — не запускалась). Её собственный id в ответ не попадает. */
    private fun checkOf(parent: QueueJobEntity, debug: Boolean, withStage: Boolean): QueueCheckDto? {
        val check = access.checkOf(parent) ?: return null
        val wait = waitEstimate(check)
        return QueueCheckDto(
            status = check.status,
            queuePosition = wait.position,
            estimatedWaitSeconds = wait.seconds,
            estimatedWaitFromSeconds = wait.fromSeconds,
            estimatedWaitToSeconds = wait.toSeconds,
            estimateReason = wait.reason,
            progressPercent = check.progress,
            phaseMessage = check.message,
            error = errorOf(check, debug),
            startedAt = check.startedAt,
            finishedAt = check.finishedAt,
            stage = if (withStage) stageOf(check) else null,
            pack = check.resultJson?.let { mapper.readTree(it) },
            artifacts = artifactsOf(check, ownerId = parent.id, debug = debug),
        )
    }

    /** Место в очереди, оценка ожидания по медиане последних задач и причина оценки (или её отсутствия). */
    private fun waitEstimate(job: QueueJobEntity): WaitEstimate {
        val ahead = if (job.status in QueueStatus.WAITING) queueSql.waitingAhead(job.createdAt, job.id) else null
        val config = platform.findById(PlatformEntity.PLATFORM_CONFIG_ID).orElseThrow()
        val configured = if (config.paused) 0 else config.maxParallel
        val slots = minOf(workers.sumFreshCapacity(WorkerNodeService.FRESH_SECONDS), configured)
        val durations = queueSql.recentSuccessDurations(job.kind, job.profileId)
        val running = jobs.countByStatusIn(QueueStatus.OCCUPYING_SLOT)
        val median = durations.medianSeconds
        val estimate = if (ahead != null && slots > 0 && durations.samples >= MIN_SAMPLES_FOR_ESTIMATE && median != null) {
            (((ahead + running).toDouble() / slots) * median).toLong().coerceAtLeast(0)
        } else {
            null
        }
        val reason = when {
            slots == 0 -> "Нет доступных исполнителей или очередь на паузе"
            estimate == null -> "Недостаточно измерений для надёжной оценки"
            else -> "Приблизительно по медиане последних задач; фактическое время может отличаться"
        }
        return WaitEstimate(ahead?.plus(1), estimate, reason)
    }

    private fun errorOf(job: QueueJobEntity, debug: Boolean): String? =
        if (debug) job.error else job.error?.substringBefore(CliErrorTranslator.DETAILS_SEPARATOR)?.take(MAX_PUBLIC_ERROR_LENGTH)

    /**
     * Артефакты в статусе; [ownerId] — id, под которым они скачиваются (у проверки — id родителя:
     * id ребёнка наружу не уходит).
     */
    private fun artifactsOf(job: QueueJobEntity, ownerId: String, debug: Boolean): List<QueueArtifactDto> =
        artifacts.findAllByJobId(job.id)
            .filter { debug || !it.debug }
            .map { QueueArtifactDto(it.id, it.downloadName, it.sizeBytes, "/api/v2/jobs/$ownerId/artifacts/${it.id}/download") }

    /** Этап пакера для статуса: прошедшее время считается сейчас (по часам API, startedAtMs — исполнителя). */
    private fun stageOf(job: QueueJobEntity): JobStageDto? {
        // Для упаковки — в любом статусе: фронт показывает пройденные этапы с длительностями и после завершения.
        if (job.kind != JobKind.PACK && job.status !in QueueStatus.EXECUTING) return null
        return job.stageJson?.let { runCatching { JobStageDto.of(mapper.readValue(it, JobStage::class.java)) }.getOrNull() }
    }

    /** Каталожный профиль — DTO из каталога (null, если исчез); разовый стол — из снимка profileJson. */
    private fun profileOf(job: QueueJobEntity, request: JobRequest): ProfileDto? =
        if (request.customBed != null) {
            runCatching { CustomProfile.dtoOf(mapper.readTree(job.profileJson)) }.getOrNull()
        } else {
            profiles.list().find { it.id == job.profileId }
        }

    /** Часы начала и конца работ расписания считаются при выдаче: в отчёте пакера есть только секунды. */
    private fun addScheduleClocks(result: JsonNode?) {
        result?.path("jobs")?.forEach { row ->
            (row as? ObjectNode)?.apply {
                put("startClock", ClockFormat.format(path("startSeconds").asLong()))
                put("endClock", ClockFormat.format(path("endSeconds").asLong()))
            }
        }
    }

    private companion object {
        const val JOBS_PAGE_SIZE = 20
        const val MIN_SAMPLES_FOR_ESTIMATE = 5
        const val MAX_PUBLIC_ERROR_LENGTH = 500
    }
}
