package com.goatpacker.web.dto

import com.fasterxml.jackson.annotation.JsonInclude

/** Принтер из каталога профилей: файл {profiles-dir}/{id}.json. */
@JsonInclude(JsonInclude.Include.NON_NULL)
data class ProfileDto(
    val id: String,
    val name: String,
    val slicerKind: String,
    val bedWidthMm: Double,
    val bedDepthMm: Double,
    val edgeMarginMm: Double,
    val supportsNozzleClean: Boolean,
)

/** Ответ на создание джоба. */
data class JobCreatedDto(
    val jobId: String,
)

/** Файл-артефакт джоба. */
data class ArtifactDto(
    val name: String,
    val sizeBytes: Long,
    val downloadUrl: String,
)

/** Итог pack-джоба. */
data class PackInfoDto(
    val objectCount: Int,
    val profileId: String,
    val clearanceMm: Double,
)

/** Работа в расписании. */
data class ScheduleJobDto(
    val sequence: Int,
    val batchSize: Int,
    val startClock: String,
    val endClock: String,
    val startSeconds: Long,
    val endSeconds: Long,
    val slicerSeconds: Long,
    val preparationSeconds: Long,
    val filamentGrams: Double,
    val cumulativeObjects: Int,
    val overnight: Boolean,
)

/** Пластина мультипластинного 3MF. */
data class PlateDto(
    val label: String,
    val objectCount: Int,
)

/** Итог schedule-джоба. */
data class ScheduleInfoDto(
    val printer: String,
    val totalObjects: Int,
    val totalFilamentGrams: Double,
    val completionClock: String,
    val jobs: List<ScheduleJobDto>,
    val plates: List<PlateDto>,
)

/** Статусная сводка джоба — то, что фронтенд поллит раз в секунду. */
@JsonInclude(JsonInclude.Include.ALWAYS)
data class JobStatusDto(
    val jobId: String,
    val kind: String,
    val status: String,
    val phase: String,
    val progressPercent: Int,
    val phaseMessage: String?,
    val error: String?,
    val createdAt: java.time.OffsetDateTime,
    val finishedAt: java.time.OffsetDateTime?,
    val artifacts: List<ArtifactDto>,
    val pack: PackInfoDto?,
    val schedule: ScheduleInfoDto?,
)
