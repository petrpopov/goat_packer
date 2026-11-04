package com.goatpacker.web.model

/** Допустимые диапазоны настроек задач (мм, часы, секунды, проценты — как в API и в документации). */
object JobLimits {
    val CLEARANCE_MM = 0.05..10.0
    val RESOLUTION_MM = 0.05..1.0
    val ATTEMPTS = 1L..1_000_000L
    val EDGE_MARGIN_MM = 0.0..20.0
    val TOWER_WIDTH_MM = 5.0..60.0
    val TIME_WINDOW_HOURS = 0.5..240.0
    val CHANGE_SECONDS = 0.0..600.0
    val TIME_MARGIN_PCT = 0.0..50.0
    val SUPPORT_OVERLAP_MM = 0.0..20.0
    val SCHEDULE_MINUTES = 0..600
}

/** Конечное число из диапазона; null (поле не задано) считается допустимым. */
fun Double?.isOutOf(range: ClosedFloatingPointRange<Double>): Boolean = this != null && (!isFinite() || this !in range)
