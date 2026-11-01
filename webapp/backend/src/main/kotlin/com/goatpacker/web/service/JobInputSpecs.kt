package com.goatpacker.web.service

/** Формат `--nozzle-clean`: `global`, `from-z=H`, с необязательным `,every=N` (H в мм, N ≥ 1). */
object NozzleCleanSpec {
    const val OFF = "off"
    const val EVERY_KEY = ",every="
    const val FROM_Z_KEY = "from-z="

    val PATTERN = Regex("""^(global|from-z=\d+(\.\d+)?)(,every=[1-9]\d*)?$""")
    private val FROM_Z = Regex("""from-z=(\d+(\.\d+)?)""")

    /** Чистка выключена: пусто или `off`. */
    fun isOff(spec: String): Boolean = spec.isEmpty() || spec.equals(OFF, ignoreCase = true)

    /** Высота H из `from-z=H`; null — спека без неё. */
    fun fromZ(spec: String): Double? = FROM_Z.find(spec)?.groupValues?.get(1)?.toDoubleOrNull()

    /** Период N из `every=N` как текст; пусто — без периода. */
    fun every(spec: String): String = spec.substringAfter(EVERY_KEY, "")
}

/** Время суток расписания: часы 0..[MAX_HOURS] (расписание может уходить за полночь), минуты 00..59. */
data class ClockTime(val hours: Int, val minutes: Int) {
    val totalMinutes: Int get() = hours * MINUTES_PER_HOUR + minutes

    companion object {
        const val MAX_HOURS = 47
        private const val MINUTES_PER_HOUR = 60
        private val PATTERN = Regex("""^(\d{1,2}):([0-5]\d)$""")

        /** Разбирает `ЧЧ:ММ`; null — формат нарушен. Диапазон часов проверяет вызывающий. */
        fun parse(raw: String): ClockTime? =
            PATTERN.matchEntire(raw)?.let { ClockTime(it.groupValues[1].toInt(), it.groupValues[2].toInt()) }
    }
}
