package com.goatpacker.web.service

/**
 * Часовые метки расписания: секунды от старта горизонта → "ЧЧ:ММ", при выходе за первые
 * сутки — префикс "D+N" ("D+1 12:26"), как в clock_label самого daily_scheduler.
 */
object ClockFormat {

    fun format(seconds: Long): String {
        val day = seconds / 86_400
        val within = seconds % 86_400
        val hours = within / 3_600
        val minutes = (within % 3_600) / 60

        return if (day == 0L) {
            String.format("%02d:%02d", hours, minutes)
        } else {
            "D+$day " + String.format("%02d:%02d", hours, minutes)
        }
    }
}
