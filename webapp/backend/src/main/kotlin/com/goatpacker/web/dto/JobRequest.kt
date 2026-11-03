package com.goatpacker.web.dto

import com.goatpacker.web.model.JobKind
import com.goatpacker.web.model.PackDefaults
import com.goatpacker.web.model.ScheduleDefaults

/**
 * Тело `POST /api/v2/jobs`. Сохраняется в `queue_jobs.params_json` как есть, поэтому имена полей и
 * дефолты — часть формата данных: воркер читает ту же структуру.
 */
data class JobRequest(
    val kind: String = JobKind.PACK.name,
    val fileId: String? = null,
    val packJobId: String? = null,
    val profileId: String = "",
    val clearance: Double = PackDefaults.CLEARANCE_MM,
    val resolution: Double = PackDefaults.RESOLUTION_MM,
    val attempts: Long = PackDefaults.ATTEMPTS,
    val edgeMargin: Double? = null,
    val layout: String = PackDefaults.LAYOUT,
    val nozzleClean: String = "",
    val dayStart: String = ScheduleDefaults.DAY_START,
    val nightStart: String = ScheduleDefaults.NIGHT_START,
    val horizonEnd: String = ScheduleDefaults.HORIZON_END,
    val turnaroundMin: Int = ScheduleDefaults.TURNAROUND_MIN,
    val prepMin: Int = ScheduleDefaults.PREP_MIN,
    val benchMode: String = ScheduleDefaults.BENCH_MODE_ADAPTIVE,
    /** Окно печати, ч (упаковка); null = не задано. */
    val timeWindowHours: Double? = null,
    /** Коррекция смены прутка AMS (упаковка и расписание). */
    val amsCorrection: Boolean = false,
    val changeSeconds: Double? = null,
    val timeMarginPct: Double? = null,
    /** Перекрытие поддержек (упаковка), мм; null = без ограничений. */
    val supportOverlapMm: Double? = null,
    /** Вариант теста смены AMS (калибровка). */
    val variant: String = ScheduleDefaults.CALIBRATION_VARIANT_NORMAL,
    /** «Моего принтера нет в списке» (только упаковка): вместо profileId; задача получает разовый профиль. */
    val customBed: CustomBed? = null,
    /** false — упаковка без полной нарезки (по пробной нарезке нескольких объектов); полная — доводкой. */
    val finalSlice: Boolean = true,
    /** Доводка «Нарезать стол» (PACK с packJobId): блоб состояния родителя. Ставит только сервер. */
    val finishStateBlobId: String? = null,
    /** Слайсер, в котором пользователь печатает STL (bambu|orca); null — умолчание профиля. Для 3MF не задаётся. */
    val slicer: String? = null,
)
