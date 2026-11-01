package com.goatpacker.web.dto

import com.fasterxml.jackson.annotation.JsonInclude
import com.fasterxml.jackson.databind.JsonNode
import com.goatpacker.web.model.JobStage
import com.goatpacker.web.model.ProjectBed
import com.goatpacker.web.model.StageSpan
import com.goatpacker.web.model.UploadKind
import java.time.OffsetDateTime

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
    /** printable_area профиля [x0, y0, x1, y1], мм, координаты стола. */
    val printableArea: List<Double> = emptyList(),
    /** exclusion_rectangles профиля — зоны, где печатать нельзя (у P1S угол 18×28). */
    val exclusionRectangles: List<List<Double>> = emptyList(),
    /** Упрощённый контур пластины для сцены; null — фронт рисует прямоугольник области. */
    val plateOutline: List<List<Double>>? = null,
    /**
     * Производитель для группировки в выборе принтера: поле vendor профиля, иначе printer_name
     * без последнего слова (у импортированных из Orca vendor есть всегда; у ручных P1S/A2L/U1 он
     * прописан явно — для многословных моделей правило «минус слово» ошиблось бы).
     */
    val vendor: String = "",
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

/**
 * Проверка нарезкой внутри pack-джоба (локальный режим; в серверном та же форма — [QueueCheckDto]). Id внутренней
 * задачи наружу не уходит: [artifacts] скачиваются по id самой упаковки.
 */
@JsonInclude(JsonInclude.Include.ALWAYS)
data class CheckStatusDto(
    val status: String,
    val queuePosition: Long?,
    val progressPercent: Int,
    val phaseMessage: String?,
    val error: String?,
    val startedAt: OffsetDateTime?,
    val finishedAt: OffsetDateTime?,
    val stage: JobStageDto?,
    val pack: PackInfoDto?,
    val artifacts: List<ArtifactDto>,
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
    /** false — стол упакован по пробной нарезке, полная нарезка ещё не запускалась. */
    val finalSliced: Boolean = true,
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
    /** Все заходы в этапы по порядку; фронт показывает пройденные этапы с длительностями и после завершения. */
    val history: List<StageSpan> = emptyList(),
) {
    companion object {
        private const val MILLIS_PER_SECOND = 1000

        fun of(stage: JobStage, nowMs: Long = System.currentTimeMillis()) = JobStageDto(
            id = stage.id,
            startedAtMs = stage.startedAtMs,
            elapsedSeconds = ((nowMs - stage.startedAtMs) / MILLIS_PER_SECOND).coerceAtLeast(0),
            expectedSeconds = stage.expectedSeconds,
            objects = stage.objects,
            attempt = stage.attempt,
            visited = stage.visited,
            detail = stage.detail,
            history = stage.history,
        )
    }
}

/**
 * Настройки pack-джоба как отправлены (для режима «только чтение» формы при открытии по URL).
 * [layout] search|grid; [nozzleClean] сырая спека, "" = выключена; [customBed] — свой стол или null.
 */
@JsonInclude(JsonInclude.Include.ALWAYS)
data class PackParamsDto(
    val clearance: Double,
    val resolution: Double,
    val layout: String,
    val nozzleClean: String,
    val edgeMargin: Double?,
    val finalSlice: Boolean,
    val timeWindowHours: Double?,
    val timeMarginPct: Double?,
    val amsCorrection: Boolean,
    val changeSeconds: Double?,
    val supportOverlapMm: Double?,
    val customBed: CustomBed?,
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
    val createdAt: OffsetDateTime,
    val finishedAt: OffsetDateTime?,
    val artifacts: List<ArtifactDto>,
    val pack: PackInfoDto?,
    val schedule: ScheduleInfoDto?,
    val calibration: CalibrationInfoDto? = null,
    /** Этап упаковки; null — не pack-джоб или этап ещё не начался. */
    val stage: JobStageDto? = null,
    /** Последняя раскладка пакера (layout-событие как есть); есть и после успеха. null — раскладки нет. */
    val layout: JsonNode? = null,
    /** Имя загруженного файла; только pack-джоб (иначе null). */
    val originalName: String? = null,
    /** Профиль принтера, как элемент /profiles (для своего стола — разовый, id `custom`); только pack. */
    val profile: ProfileDto? = null,
    /** Настройки упаковки как отправлены; только pack. */
    val params: PackParamsDto? = null,
    /** Проверка нарезкой стола внутри этой упаковки; null — не запускалась. */
    val check: CheckStatusDto? = null,
)

/** Ответ /api/profiles/detect: profileId = null, когда по файлу определить не удалось; printerModel — что за принтер записан в файле (для понятных сообщений). */
@JsonInclude(JsonInclude.Include.ALWAYS)
data class DetectProfileDto(
    val profileId: String?,
    val printerModel: String?,
    /** "3mf" — проект слайсера, "stl" — сетка (не разбирается, принтер выбирает пользователь). */
    val kind: String = UploadKind.PROJECT_3MF,
    /** Стол из настроек 3MF, когда принтер не распознан (profileId = null); иначе null. */
    val projectBed: ProjectBed? = null,
)
