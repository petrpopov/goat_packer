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
    /**
     * Реальная длительность смены прутка AMS, с (`filament_change_seconds` профиля).
     * null — у принтера нет AMS-смен (тулчейнджер), коррекция времени ему не нужна.
     */
    val filamentChangeSeconds: Double? = null,
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
    /** Сводка блока `time` из packed.verification.json; null — время не измерено. */
    val time: PackTimeDto? = null,
    /** У модели обнаружены поддержки — поворот заблокирован; null — пакер поле не прислал. */
    val supportsDetected: Boolean? = null,
    /** Допустимое перекрытие поддержек, мм; null — без ограничений. */
    val supportOverlapMm: Double? = null,
)

/** Итог calibration-джоба: значения calibration.json и текст instructions.md. */
data class CalibrationInfoDto(
    val profileId: String,
    val variant: String,
    val changesExpected: Int,
    val changesSliced: Int,
    val estimateASeconds: Double,
    val estimateBSeconds: Double,
    val slicerChangeSeconds: Double,
    val instructions: String,
)

/** Оценка времени печати записанного стола (с коррекцией смен прутка и запасом). */
data class PackTimeDto(
    /** Окно печати, ч; null — окно не задавалось. */
    val windowHours: Double?,
    val marginPct: Double,
    /** Реальная длительность смены, с; null — коррекция смен выключена. */
    val realChangeSeconds: Double?,
    /** Объектов на записанном столе. */
    val chosenCount: Int,
    /** Объектов на полном (максимальном) столе — наибольший count среди замеров. */
    val fullCount: Int,
    /** Скорректированное время записанного стола, с. */
    val correctedSeconds: Double,
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

/**
 * Текущий этап упаковки для степпера и строки статуса. [elapsedSeconds] посчитан при выдаче
 * статуса (фронт досчитывает время локально между опросами).
 */
data class JobStageDto(
    val id: String,
    val startedAtMs: Long,
    val elapsedSeconds: Long,
    val expectedSeconds: Double?,
    val objects: Int?,
    val attempt: Int,
    val visited: List<String>,
    val detail: String?,
) {
    companion object {
        fun of(stage: com.goatpacker.web.job.JobStage, nowMs: Long = System.currentTimeMillis()) = JobStageDto(
            id = stage.id,
            startedAtMs = stage.startedAtMs,
            elapsedSeconds = ((nowMs - stage.startedAtMs) / 1000).coerceAtLeast(0),
            expectedSeconds = stage.expectedSeconds,
            objects = stage.objects,
            attempt = stage.attempt,
            visited = stage.visited,
            detail = stage.detail,
        )
    }
}

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
    val calibration: CalibrationInfoDto? = null,
    /** Этап упаковки; null — не pack-джоб или этап ещё не начался. */
    val stage: JobStageDto? = null,
)
