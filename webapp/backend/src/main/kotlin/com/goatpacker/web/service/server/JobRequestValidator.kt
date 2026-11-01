package com.goatpacker.web.service.server

import com.goatpacker.web.dto.JobRequest
import com.goatpacker.web.exception.badRequest
import com.goatpacker.web.model.JobKind
import com.goatpacker.web.model.JobLimits
import com.goatpacker.web.model.PackDefaults
import com.goatpacker.web.model.ScheduleDefaults
import com.goatpacker.web.model.isOutOf
import com.goatpacker.web.service.ClockTime
import com.goatpacker.web.service.NozzleCleanSpec

/** Проверка тела `POST /api/v2/jobs` до обращения к БД: форма запроса и диапазоны настроек. */
object JobRequestValidator {
    private val LAYOUTS = setOf(PackDefaults.LAYOUT, "grid")
    private val BENCH_MODES = setOf(ScheduleDefaults.BENCH_MODE_ADAPTIVE, ScheduleDefaults.BENCH_MODE_FULL)
    private val CALIBRATION_VARIANTS = setOf(ScheduleDefaults.CALIBRATION_VARIANT_NORMAL, ScheduleDefaults.CALIBRATION_VARIANT_PERFECTIONIST)

    /** Возвращает тип задачи; нарушение — 400 с понятным текстом. */
    fun validate(r: JobRequest): JobKind {
        val kind = JobKind.entries.find { it.name == r.kind }
        if (kind == null || (r.fileId == null) == (r.packJobId == null)) {
            badRequest("Укажите один источник и тип ${JobKind.entries.joinToString("/")}")
        }
        if (r.finishStateBlobId != null) badRequest("finishStateBlobId задаёт сервер")
        // PACK по packJobId — доводка «Нарезать стол»: всё берётся из родителя.
        if (kind == JobKind.PACK && r.packJobId != null) return kind
        if (kind == JobKind.CALIBRATION && r.packJobId != null) badRequest("Для калибровки нужен загруженный проект")

        validateBed(r, kind)
        if (r.variant !in CALIBRATION_VARIANTS) badRequest("Вариант калибровки: ${CALIBRATION_VARIANTS.joinToString(" или ")}")
        if (r.supportOverlapMm.isOutOf(JobLimits.SUPPORT_OVERLAP_MM)) badRequest("Перекрытие поддержек должно быть 0–20 мм")
        if (r.clearance.isOutOf(JobLimits.CLEARANCE_MM) || r.resolution.isOutOf(JobLimits.RESOLUTION_MM)) {
            badRequest("Некорректные зазор/разрешение")
        }
        if (r.attempts !in JobLimits.ATTEMPTS || r.layout !in LAYOUTS || r.benchMode !in BENCH_MODES) badRequest("Некорректный режим поиска")
        if (r.edgeMargin.isOutOf(JobLimits.EDGE_MARGIN_MM)) badRequest("Некорректный отступ")
        if (r.timeWindowHours.isOutOf(JobLimits.TIME_WINDOW_HOURS)) badRequest("Окно печати должно быть 0.5–240 ч")
        if (r.changeSeconds.isOutOf(JobLimits.CHANGE_SECONDS)) badRequest("Длительность смены прутка должна быть 0–600 с")
        if (r.timeMarginPct.isOutOf(JobLimits.TIME_MARGIN_PCT)) badRequest("Запас времени должен быть 0–50 %")
        if (r.nozzleClean.isNotBlank()) validateNozzleClean(r.nozzleClean)
        validateScheduleWindow(r)
        return kind
    }

    private fun validateBed(r: JobRequest, kind: JobKind) {
        val bed = r.customBed
        if (bed != null) {
            if (kind != JobKind.PACK) badRequest("Свой стол пока доступен только для упаковки")
            if (r.profileId.isNotBlank()) badRequest("Укажите либо принтер из каталога, либо свой стол")
            bed.validate()
        } else if (r.profileId.isBlank()) {
            badRequest("Выберите принтер или укажите свой стол")
        }
    }

    private fun validateNozzleClean(spec: String) {
        if (!NozzleCleanSpec.PATTERN.matches(spec)) badRequest("Некорректная чистка сопла")
        if (spec.startsWith(NozzleCleanSpec.FROM_Z_KEY) && NozzleCleanSpec.fromZ(spec)?.let { it.isFinite() && it > 0 } != true) {
            badRequest("Высота должна быть положительной")
        }
        if (NozzleCleanSpec.EVERY_KEY in spec && NozzleCleanSpec.every(spec).toLongOrNull() == null) {
            badRequest("Слишком большой период очистки")
        }
    }

    private fun validateScheduleWindow(r: JobRequest) {
        val day = clockMinutes(r.dayStart)
        val night = clockMinutes(r.nightStart)
        val horizon = clockMinutes(r.horizonEnd)
        if (!(day < night && night < horizon) || r.prepMin !in JobLimits.SCHEDULE_MINUTES || r.turnaroundMin !in JobLimits.SCHEDULE_MINUTES) {
            badRequest("Некорректное окно времени")
        }
    }

    private fun clockMinutes(raw: String): Int {
        val clock = ClockTime.parse(raw) ?: badRequest("Формат времени ЧЧ:ММ")
        if (clock.hours > ClockTime.MAX_HOURS) badRequest("Часы 0–${ClockTime.MAX_HOURS}")
        return clock.totalMinutes
    }
}
