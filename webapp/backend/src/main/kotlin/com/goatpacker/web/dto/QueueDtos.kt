package com.goatpacker.web.dto

import com.fasterxml.jackson.annotation.JsonUnwrapped
import com.fasterxml.jackson.databind.JsonNode
import com.goatpacker.web.entity.QueueStatus
import com.goatpacker.web.entity.UploadState
import java.time.Instant

/** Ответ на постановку задачи; [readToken] выдаётся один раз — по нему задачу можно открыть чужой сессией. */
data class CreatedJobDto(val jobId: String, val readToken: String? = null)

data class AccessRequest(val jobId: String = "", val token: String = "")

data class ShareLinkDto(val token: String)

data class HealthDto(val status: String, val queue: String)

/** Ссылка на скачивание: подписанная ссылка хранилища или маршрут API, если хранилище не подписывает. */
data class DownloadLinkDto(val url: String, val filename: String)

data class QueueArtifactDto(val id: String, val name: String, val sizeBytes: Long, val downloadUrl: String)

data class UploadIntent(
    val name: String = "",
    val sizeBytes: Long = 0,
    val challengeId: String? = null,
    val nonce: String? = null,
)

data class UploadDto(
    val fileId: String,
    val originalName: String,
    val sizeBytes: Long,
    val state: UploadState,
    val profileId: String?,
    val printerModel: String?,
    val projectBed: JsonNode?,
    val kind: String,
    val expiresAt: Instant?,
)

/** Проверка нарезкой внутри задачи: её собственный id в ответ не попадает. */
data class QueueCheckDto(
    val status: QueueStatus,
    val queuePosition: Long?,
    val estimatedWaitSeconds: Long?,
    val estimatedWaitFromSeconds: Long?,
    val estimatedWaitToSeconds: Long?,
    val estimateReason: String,
    val progressPercent: Int,
    val phaseMessage: String?,
    val error: String?,
    val startedAt: Instant?,
    val finishedAt: Instant?,
    val stage: JobStageDto?,
    val pack: JsonNode?,
    val artifacts: List<QueueArtifactDto>,
)

/** Строка списка задач и общая часть статуса одной задачи. */
data class QueueJobSummaryDto(
    val canManage: Boolean,
    val jobId: String,
    val kind: String,
    val status: QueueStatus,
    val phase: String,
    val progressPercent: Int,
    val phaseMessage: String?,
    val error: String?,
    val createdAt: Instant,
    val finishedAt: Instant?,
    val expiresAt: Instant?,
    val originalName: String,
    val queuePosition: Long?,
    val estimatedWaitSeconds: Long?,
    val estimatedWaitFromSeconds: Long?,
    val estimatedWaitToSeconds: Long?,
    val estimateReason: String,
    val artifacts: List<QueueArtifactDto>,
    val pack: JsonNode?,
    val schedule: JsonNode?,
    val calibration: JsonNode?,
    val stage: JobStageDto?,
    val check: QueueCheckDto?,
)

/** Статус одной задачи: сводка плюс раскладка, профиль и настройки (в список они не входят — он должен быть лёгким). */
data class QueueJobDetailDto(
    @get:JsonUnwrapped val summary: QueueJobSummaryDto,
    val layout: JsonNode?,
    val profile: ProfileDto?,
    val params: PackParamsDto?,
)

data class QueueJobPageDto(val items: List<QueueJobSummaryDto>, val total: Long)
