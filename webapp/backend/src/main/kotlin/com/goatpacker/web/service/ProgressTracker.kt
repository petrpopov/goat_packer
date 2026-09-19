package com.goatpacker.web.service

import com.goatpacker.web.job.JobKind
import com.goatpacker.web.job.JobPhase

/** Результат разбора одной строки вывода бинарника: фаза, процент, сообщение. */
data class ProgressEvent(
    val phase: JobPhase,
    val progressPercent: Int,
    val phaseMessage: String,
)

/**
 * Чистый разбор строки вывода бинарника → (фаза, процент) | null.
 *
 * Пакер:
 *  - "probing prime tower position" → probe, 5
 *  - "packing (N attempts..." → packing, 10
 *  - "pack_footprint: X/N attempts..." → packing, 10 + 60·X/N (N может быть меньше X при раннем
 *    стопе — кламп 0..100); ПОСЛЕ начала repack-прохода та же строка → repacking, 40 + 30·X/N
 *  - "best layout:" → assembling, 72
 *  - "assembling 3MF" → assembling, 75
 *  - "slicing final plate" / "slicing repacked plate" → slicing, 80
 *  - "repack" в любой форме ("re-packing", "repacking once") → repacking, 40
 *  - "wrote" + путь .3mf → cleanup, 95
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

    private val PACK_FOOTPRINT_ATTEMPTS = Regex("""pack_footprint:\s*(\d+)/(\d+)\s+attempts""")
    internal val SLICING_SAMPLE = Regex("""slicing sample (\d+)/(\d+)""")
    private val PROBE_OBJECTS = Regex("""(?::\s*\d+\s+objects)\s*$""")
    private val REFINING = Regex("""refining: slicing batch""")

    /**
     * Возвращает (фаза, процент 0..100) или null, если строка не распознана.
     *
     * [lastSampleTotal] — последний знаменатель j из строк "slicing sample i/j" (состояние трекера);
     * используется правилом "refining". [inRepack] — уже начался ли repack-проход пакера: те же
     * строки pack_footprint тогда относятся ко второй упаковке и маппятся в фазу repacking.
     */
    fun parse(
        kind: JobKind,
        line: String,
        lastSampleTotal: Int? = null,
        inRepack: Boolean = false,
    ): Pair<JobPhase, Int>? {
        val lower = line.lowercase()

        return when (kind) {
            JobKind.PACK -> parsePacker(lower, line, inRepack)
            JobKind.SCHEDULE -> parseScheduler(lower, line, lastSampleTotal)
        }
    }

    private fun parsePacker(lower: String, line: String, inRepack: Boolean): Pair<JobPhase, Int>? {
        // "slicing repacked plate" — это уже слайсинг результата перепаковки, а не сама
        // перепаковка; проверяем до правила "repack", которое матчила бы слово "repacked".
        if (lower.startsWith("slicing repacked plate")) {
            return JobPhase.SLICING to 80
        }
        PACK_FOOTPRINT_ATTEMPTS.find(line)?.let { match ->
            val done = match.groupValues[1].toLong()
            val total = match.groupValues[2].toLong().coerceAtLeast(1)

            return if (inRepack) {
                JobPhase.REPACKING to (40.0 + 30.0 * done / total).toInt().coerceIn(0, 100)
            } else {
                JobPhase.PACKING to (10.0 + 60.0 * done / total).toInt().coerceIn(0, 100)
            }
        }
        // "repack" проверяем раньше "packing (": строки вроде "re-packing (2000 attempts..."
        // иначе были бы распознаны как старт первой упаковки.
        if ("repack" in lower || "re-pack" in lower) {
            return JobPhase.REPACKING to 40
        }
        if ("probing prime tower position" in lower) {
            return JobPhase.PROBE to 5
        }
        if ("packing (" in lower && "attempts" in lower) {
            return JobPhase.PACKING to 10
        }
        if ("best layout:" in lower) {
            return JobPhase.ASSEMBLING to 72
        }
        if ("assembling 3mf" in lower) {
            return JobPhase.ASSEMBLING to 75
        }
        if ("slicing final plate" in lower) {
            return JobPhase.SLICING to 80
        }
        if ("wrote" in lower && ".3mf" in lower) {
            return JobPhase.CLEANUP to 95
        }

        return null
    }

    private fun parseScheduler(lower: String, line: String, lastSampleTotal: Int?): Pair<JobPhase, Int>? {
        // Summary-строка ("...: 67 objects, 3 jobs [...") содержит и ": N objects", и "jobs [",
        // поэтому проверяется первой — иначе сработало бы правило probe.
        if ("objects, " in lower && "jobs [" in lower) {
            return JobPhase.OPTIMIZING to 95
        }
        if ("measured sizes cover batches" in lower) {
            return JobPhase.OPTIMIZING to 78
        }
        if ("writing" in lower && "plate(s)" in lower) {
            return JobPhase.WRITING to 90
        }
        SLICING_SAMPLE.find(line)?.let { match ->
            val index = match.groupValues[1].toLong()
            val total = match.groupValues[2].toLong().coerceAtLeast(1)
            val percent = 10.0 + 65.0 * index / total

            return JobPhase.BENCHMARK to percent.toInt().coerceIn(0, 100)
        }
        if (REFINING.containsMatchIn(lower)) {
            return JobPhase.BENCHMARK to refiningPercent(lastSampleTotal)
        }
        if (PROBE_OBJECTS.containsMatchIn(line)) {
            return JobPhase.PROBE to 5
        }

        return null
    }

    private fun refiningPercent(lastTotal: Int?): Int {
        val k = lastTotal?.takeIf { it > 0 } ?: return 11

        return (10.0 + 65.0 * (k + 1) / k).toInt().coerceIn(0, 100)
    }
}

/**
 * Состоянийный трекер прогресса одного джоба: оборачивает [ProgressRules] и гарантирует, что
 * прогресс не откатывается назад (явное исключение — фаза repacking, она закономерно сбрасывает
 * процент на 40). Процент клампится в 0..99, пока процесс жив; 100 выставляет JobRunner при exit 0.
 */
class ProgressTracker(private val kind: JobKind) {
    var phase: JobPhase = JobPhase.UPLOAD
        private set

    var percent: Int = 0
        private set

    /** Последнее значимое сообщение (строка вывода «как есть», с префиксом внутри repack). */
    var message: String? = null
        private set

    /** Последний j из строк "slicing sample i/j" — для правила "refining". */
    private var lastSampleTotal: Int? = null

    /** Начался ли repack-проход пакера (один максимум): строки pack_footprint тогда его прогресс. */
    private var inRepack: Boolean = false

    /**
     * Обрабатывает очередную строку вывода. Возвращает событие для статусной сводки: фаза/процент
     * могут остаться прежними (строка не распознана или нарушила монотонность), но сообщение —
     * всегда последняя значимая строка вывода (внутри repack — с префиксом «Перепаковка:»).
     */
    fun onLine(line: String): ProgressEvent {
        if (line.isBlank()) {
            return ProgressEvent(phase, percent, message.orEmpty())
        }

        if (kind == JobKind.PACK) {
            val lower = line.lowercase()
            if ("repack" in lower || "re-pack" in lower) {
                inRepack = true
            }
        }

        val parsed = ProgressRules.parse(kind, line, lastSampleTotal, inRepack)
        if (kind == JobKind.SCHEDULE) {
            ProgressRules.SLICING_SAMPLE.find(line)?.groupValues?.get(2)?.toIntOrNull()
                ?.let { lastSampleTotal = it }
        }

        if (parsed != null) {
            val (candidatePhase, rawPercent) = parsed
            val candidatePercent = rawPercent.coerceIn(0, 99)
            // Монотонность: прогресс не откатывается, кроме явного repacking.
            if (candidatePhase == JobPhase.REPACKING || candidatePercent >= percent) {
                phase = candidatePhase
                percent = candidatePercent
            }
        }

        message = if (inRepack && kind == JobKind.PACK && PACK_FOOTPRINT_ATTEMPTS_RE.containsMatchIn(line)) {
            "Перепаковка: $line"
        } else {
            line
        }

        return ProgressEvent(phase, percent, message ?: line)
    }

    private companion object {
        private val PACK_FOOTPRINT_ATTEMPTS_RE = Regex("""pack_footprint:\s*(\d+)/(\d+)\s+attempts""")
    }
}
