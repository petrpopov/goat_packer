package com.goatpacker.web.model

/** Результат pack-джоба, распарсенный из verification.json после успеха. */
data class PackJobResult(
    val objectCount: Int,
    val profileId: String,
    val clearanceMm: Double,
    /** Оценка времени из блока `time` verification.json; null — не измерено. */
    val time: PackTimeResult? = null,
    /**
     * `supports_detected` отчёта: у модели есть поддержки за пределами силуэта, поворот
     * заблокирован. null — старый пакер без этого поля.
     */
    val supportsDetected: Boolean? = null,
    /** `support_overlap_mm` отчёта: допустимое перекрытие поддержек, мм; null — без ограничений. */
    val supportOverlapMm: Double? = null,
    /** `final_sliced` отчёта: false — полная нарезка стола ещё не запускалась («Нарезать стол»). */
    val finalSliced: Boolean = true,
    /** `tower` отчёта: башня очистки, которую пакер поставил на стол; null — башни нет (или старый пакер). */
    val tower: PackTowerResult? = null,
)

/**
 * Башня очистки стола (`tower.anchor`/`tower.size` отчёта, мм, координаты стола). Bambu Studio и
 * Snapmaker Orca при открытии проекта сдвигают башню, поэтому интерфейс просит вернуть её сюда.
 */
data class PackTowerResult(
    val anchorXMm: Double,
    val anchorYMm: Double,
    val widthMm: Double?,
    val depthMm: Double?,
)

/** Сводка блока `time` отчёта упаковщика (секунды). */
data class PackTimeResult(
    val windowHours: Double?,
    val marginPct: Double,
    val realChangeSeconds: Double?,
    val chosenCount: Int,
    val fullCount: Int,
    val correctedSeconds: Double,
)

/**
 * Результат calibration-джобы: calibration.json бинарника ams_calibration + текст
 * instructions.md (небольшой — отдаётся прямо в статусе, чтобы фронт не тянул файл отдельно).
 */
data class CalibrationJobResult(
    val profileId: String,
    val variant: String,
    val changesExpected: Int,
    val changesSliced: Int,
    val estimateASeconds: Double,
    val estimateBSeconds: Double,
    val slicerChangeSeconds: Double,
    val instructions: String,
)

/** Одна работа в расписании (из report.json → variable_batch_schedule.jobs). */
data class ScheduleJobRecord(
    val sequence: Int,
    val batchSize: Int,
    val startSeconds: Long,
    val endSeconds: Long,
    val slicerSeconds: Long,
    val preparationSeconds: Long,
    val filamentGrams: Double,
    val cumulativeObjects: Int,
    val overnight: Boolean,
)

/** Одна пластина мультипластинного 3MF (уникальный batch_size в порядке появления). */
data class PlateRecord(
    val label: String,
    val objectCount: Int,
)

/** Результат schedule-джобы, распарсенный из report.json после успеха. */
data class ScheduleJobResult(
    val printer: String,
    val totalObjects: Int,
    val totalFilamentGrams: Double,
    val completionClock: String,
    val jobs: List<ScheduleJobRecord>,
    val plates: List<PlateRecord>,
)
