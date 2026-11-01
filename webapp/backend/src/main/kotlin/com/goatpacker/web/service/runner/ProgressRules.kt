package com.goatpacker.web.service.runner

import com.goatpacker.web.model.JobKind
import com.goatpacker.web.model.JobPhase

/**
 * Разбор строк планировщика и калибровки → (фаза, процент) | null. Пакер разбирается отдельно
 * ([PackStage]/[PackStageRules]): у него этапы и время, а не фиксированные проценты.
 *
 * Планировщик:
 *  - ": N objects" в конце строки → probe, 5
 *  - "slicing sample i/j: batch" → benchmark, 10 + 65·i/j
 *  - "refining: slicing batch" → benchmark, 10 + 65·(k+1)/k, где k — последний j из строк
 *    "slicing sample" (небольшое продвижение вперёд, не убывает)
 *  - "measured sizes cover batches" → optimizing, 78
 *  - "writing ... plate(s)" → writing, 90
 *  - summary "objects, " и "jobs [" → optimizing, 95
 */
object ProgressRules {
    internal val SLICING_SAMPLE = Regex("""slicing sample (\d+)/(\d+)""")
    private val PROBE_OBJECTS = Regex("""(?::\s*\d+\s+objects)\s*$""")
    private val REFINING = Regex("""refining: slicing batch""")

    private const val PROBE_PERCENT = 5
    private const val BENCHMARK_BASE_PERCENT = 10.0
    private const val BENCHMARK_SPAN_PERCENT = 65.0
    private const val REFINING_FALLBACK_PERCENT = 11
    private const val MEASURED_SIZES_PERCENT = 78
    private const val WRITING_PERCENT = 90
    private const val SUMMARY_PERCENT = 95
    private const val CALIBRATION_REPORT_PERCENT = 95
    private const val MAX_PERCENT = 100

    /**
     * Возвращает (фаза, процент 0..100) или null, если строка не распознана (для пакера — всегда
     * null: его прогресс считает [ProgressTracker] по этапам).
     *
     * [lastSampleTotal] — последний знаменатель j из строк "slicing sample i/j" (состояние трекера);
     * используется правилом "refining".
     */
    fun parse(kind: JobKind, line: String, lastSampleTotal: Int? = null): Pair<JobPhase, Int>? {
        val lower = line.lowercase()

        return when (kind) {
            JobKind.PACK -> null
            JobKind.SCHEDULE -> parseScheduler(lower, line, lastSampleTotal)
            // ams_calibration молчит до конца; итоговый calibration.json печатается в stdout.
            JobKind.CALIBRATION -> if ("\"changes_sliced\"" in line) JobPhase.WRITING to CALIBRATION_REPORT_PERCENT else null
        }
    }

    private fun parseScheduler(lower: String, line: String, lastSampleTotal: Int?): Pair<JobPhase, Int>? {
        // Summary-строка ("...: 67 objects, 3 jobs [...") содержит и ": N objects", и "jobs [",
        // поэтому проверяется первой — иначе сработало бы правило probe.
        if ("objects, " in lower && "jobs [" in lower) return JobPhase.OPTIMIZING to SUMMARY_PERCENT
        if ("measured sizes cover batches" in lower) return JobPhase.OPTIMIZING to MEASURED_SIZES_PERCENT
        if ("writing" in lower && "plate(s)" in lower) return JobPhase.WRITING to WRITING_PERCENT
        SLICING_SAMPLE.find(line)?.let { match ->
            val index = match.groupValues[1].toLong()
            val total = match.groupValues[2].toLong().coerceAtLeast(1)
            val percent = BENCHMARK_BASE_PERCENT + BENCHMARK_SPAN_PERCENT * index / total

            return JobPhase.BENCHMARK to percent.toInt().coerceIn(0, MAX_PERCENT)
        }
        if (REFINING.containsMatchIn(lower)) return JobPhase.BENCHMARK to refiningPercent(lastSampleTotal)
        if (PROBE_OBJECTS.containsMatchIn(line)) return JobPhase.PROBE to PROBE_PERCENT

        return null
    }

    private fun refiningPercent(lastTotal: Int?): Int {
        val total = lastTotal?.takeIf { it > 0 } ?: return REFINING_FALLBACK_PERCENT

        return (BENCHMARK_BASE_PERCENT + BENCHMARK_SPAN_PERCENT * (total + 1) / total).toInt().coerceIn(0, MAX_PERCENT)
    }
}
