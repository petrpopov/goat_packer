package com.goatpacker.web.util

/**
 * Часовые метки расписания: секунды от старта горизонта → "ЧЧ:ММ", при выходе за первые
 * сутки — префикс "D+N" ("D+1 12:26"), как в clock_label самого daily_scheduler.
 */
object ClockFormat {
    fun format(seconds: Long): String {
        val day = seconds / SECONDS_PER_DAY
        val within = seconds % SECONDS_PER_DAY
        val hours = within / SECONDS_PER_HOUR
        val minutes = (within % SECONDS_PER_HOUR) / SECONDS_PER_MINUTE
        val clock = String.format("%02d:%02d", hours, minutes)

        return if (day == 0L) clock else "D+$day $clock"
    }
}
