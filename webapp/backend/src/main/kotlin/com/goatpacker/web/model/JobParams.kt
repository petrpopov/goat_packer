package com.goatpacker.web.model

import com.goatpacker.web.dto.CustomBed
import java.nio.file.Path

/** Значения по умолчанию упаковки: совпадают с дефолтами CLI colored_3mf_packer. */
object PackDefaults {
    const val CLEARANCE_MM = 0.7
    const val RESOLUTION_MM = 0.25
    const val LAYOUT = "search"

    /**
     * Дефолт CLI. Фронт поле больше не шлёт: с поддержками поиск всё равно перебирает
     * фиксированные 64 варианта, ползунок усилия ни на что не влиял. Поле принимается
     * ради совместимости API.
     */
    const val ATTEMPTS = 16_000L
    const val CANDIDATES = 2_400L
    const val SEED = 20_270_831L
}

/** Значения по умолчанию расписания и калибровки. */
object ScheduleDefaults {
    const val DAY_START = "10:00"
    const val NIGHT_START = "23:00"
    const val HORIZON_END = "34:00"
    const val TURNAROUND_MIN = 10
    const val PREP_MIN = 10
    const val BENCH_MODE_ADAPTIVE = "adaptive"
    const val BENCH_MODE_FULL = "full"
    const val CALIBRATION_VARIANT_NORMAL = "normal"
    const val CALIBRATION_VARIANT_PERFECTIONIST = "perfectionist"
}

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
    /** --support-overlap-mm; null = без ограничений (флаг не передаётся). */
    val supportOverlapMm: Double? = null,
    /** --tower-width: ширина черновой башни, мм (рёбра выключены); null = дефолт пакера (20 мм). */
    val towerWidth: Double? = null,
    /**
     * Вход — STL: команда source_project (STL → input.3mf), выполняется перед пакером в том же
     * процессе/контейнере ([ShellChain]); null — вход уже 3MF.
     */
    val sourceCommand: List<String>? = null,
    /**
     * true → --verify: стол нарезается и проверяется сразу; false — стол упакован по пробным
     * нарезкам одного объекта на класс поворота, проверку пользователь запускает позже кнопкой
     * («Нарезать стол»).
     */
    val finalSlice: Boolean = true,
    /** --finish-state: доводка упакованного стола (полная нарезка и проверки) по его состоянию. */
    val finishState: Path? = null,
    /** Свой стол, если джоб на разовом профиле: отдаётся в статусе (форма только для чтения). */
    val customBed: CustomBed? = null,
    /**
     * --slicer: слайсер, в котором пользователь печатает STL (SlicerChoice);
     * null — вход 3MF, слайсер и GUI берутся из проекта и профиля.
     */
    val slicer: String? = null,
    /** --cache-dir: каталог кэша проб (относительный — в каталоге джоба; null — пакер пробует заново). */
    val cacheDir: String? = null,
)

/** Параметры calibration-джоба — CLI-аргументы ams_calibration. */
data class CalibrationParams(
    val profileId: String,
    /** Путь к json профиля (как у упаковщика: из него бинарник берёт slicer_path). */
    val profilePath: Path,
    /** normal (10 смен) | perfectionist (30 смен). */
    val variant: String,
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
