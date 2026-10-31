package com.goatpacker.web.service

import com.fasterxml.jackson.databind.JsonNode
import com.fasterxml.jackson.databind.ObjectMapper
import com.goatpacker.web.job.JobKind
import com.goatpacker.web.job.JobPhase
import com.goatpacker.web.job.JobStage
import kotlin.math.exp

/** Результат разбора одной строки вывода бинарника: фаза, процент, сообщение, этап пакера. */
data class ProgressEvent(
    val phase: JobPhase,
    val progressPercent: Int,
    val phaseMessage: String,
    /** Текущий этап упаковки (только pack-джобы); null — этап ещё не начался или не пакер. */
    val stage: JobStage? = null,
)

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

    /**
     * Возвращает (фаза, процент 0..100) или null, если строка не распознана (для пакера — всегда
     * null: его прогресс считает [ProgressTracker] по этапам).
     *
     * [lastSampleTotal] — последний знаменатель j из строк "slicing sample i/j" (состояние трекера);
     * используется правилом "refining".
     */
    fun parse(
        kind: JobKind,
        line: String,
        lastSampleTotal: Int? = null,
    ): Pair<JobPhase, Int>? {
        val lower = line.lowercase()

        return when (kind) {
            JobKind.PACK -> null
            JobKind.SCHEDULE -> parseScheduler(lower, line, lastSampleTotal)
            // ams_calibration молчит до конца; итоговый calibration.json печатается в stdout.
            JobKind.CALIBRATION -> if ("\"changes_sliced\"" in line) JobPhase.WRITING to 95 else null
        }
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
 * Этапы colored_3mf_packer (stage-события протокола прогресса, см.
 * docs/superpowers/specs/2026-10-31-support-landing-and-progress-design.md) и их полосы на шкале.
 *
 * Полосы идут в порядке этапов и не пересекаются; перепаковка и подбор под окно времени получили
 * свои полосы ПОСЛЕ итоговой нарезки, поэтому шкала никогда не откатывается: повторные сборка и
 * нарезка после перепаковки (и каждая следующая нарезка окна) продолжают с текущего процента
 * (см. [ProgressTracker.enter]). Ширина полосы ~ доле времени этапа на сервере (проба 22 с,
 * итоговая пластина 37 объектов 317 с), поэтому итоговая нарезка — самая широкая.
 *
 * [defaultSeconds] — оценка длительности, если пакер не прислал `expected_s` (или этап распознан
 * по текстовой строке): по ней идёт интерполяция внутри этапа.
 */
enum class PackStage(
    val id: String,
    val phase: JobPhase,
    val bandStart: Int,
    val bandEnd: Int,
    private val defaultSeconds: Double,
) {
    PROBE_SLICE("probe_slice", JobPhase.PROBE, 1, 15, 30.0),
    PACKING("packing", JobPhase.PACKING, 15, 30, 60.0),
    ASSEMBLING("assembling", JobPhase.ASSEMBLING, 30, 33, 10.0),
    FINAL_SLICE("final_slice", JobPhase.SLICING, 33, 78, 300.0),
    GCODE_CHECK("gcode_check", JobPhase.SLICING, 78, 82, 30.0),
    REPACKING("repacking", JobPhase.REPACKING, 82, 86, 60.0),
    WINDOW_SLICE("window_slice", JobPhase.SLICING, 86, 95, 300.0),
    WRITING("writing", JobPhase.WRITING, 95, 99, 10.0);

    val slicing: Boolean
        get() = this == PROBE_SLICE || this == FINAL_SLICE || this == WINDOW_SLICE

    /** Ожидаемая длительность при отсутствии `expected_s`: для нарезок — по формуле из дизайна. */
    fun fallbackSeconds(objects: Int?): Double =
        if ((this == FINAL_SLICE || this == WINDOW_SLICE) && objects != null && objects > 0) {
            PROBE_SECONDS * (1 + 0.36 * (objects - 1))
        } else {
            defaultSeconds
        }

    companion object {
        /** Типичная пробная нарезка одного объекта на сервере, с (из замеров дизайна). */
        private const val PROBE_SECONDS = 22.0

        fun fromId(raw: String?): PackStage? = entries.firstOrNull { it.id == raw }
    }
}

/**
 * Резервное распознавание этапов по человекочитаемым строкам пакера — для запуска без
 * GOAT_LOG_FORMAT=json (локальный режим) и для пакеров без stage-событий. При наличии
 * stage-событий строки на этап не влияют (см. [ProgressTracker]).
 */
object PackStageRules {
    private val OBJECTS = Regex("""\((\d+) objects?\)""")
    private val PACKED_OBJECTS = Regex("""packed (\d+) objects""")

    /** (этап, число объектов | null) или null, если строка не начинает этап. */
    fun stageOf(line: String): Pair<PackStage, Int?>? {
        val lower = line.lowercase()
        val objects = OBJECTS.find(lower)?.groupValues?.get(1)?.toIntOrNull()

        return when {
            "probing prime tower position" in lower -> PackStage.PROBE_SLICE to 1
            // "slicing repacked plate" — нарезка результата перепаковки, а не сама перепаковка:
            // проверяется раньше правила "repack", которое матчило бы слово "repacked".
            lower.startsWith("slicing repacked plate") || "slicing final plate" in lower ->
                PackStage.FINAL_SLICE to objects
            "slicing time-window plate" in lower -> PackStage.WINDOW_SLICE to objects
            // "repack" раньше "packing (": "re-packing (2000 attempts…" иначе стал бы первой упаковкой.
            "repack" in lower || "re-pack" in lower -> PackStage.REPACKING to null
            "packing (" in lower && "attempts" in lower -> PackStage.PACKING to null
            "assembling 3mf" in lower ->
                PackStage.ASSEMBLING to PACKED_OBJECTS.find(lower)?.groupValues?.get(1)?.toIntOrNull()
            "wrote" in lower && ".3mf" in lower -> PackStage.WRITING to null
            else -> null
        }
    }
}

/**
 * Состоянийный трекер прогресса одного джоба. Процент клампится в 0..99, пока процесс жив;
 * 100 выставляет JobRunner при exit 0.
 *
 * Пакер: процент = полоса текущего этапа ([PackStage]) + доля внутри неё. Доля — по времени
 * (прошло / ожидается, асимптотически не доходит до конца полосы, пока этап не сменится), для
 * упаковки — ещё и по строкам `pack_footprint: X/N`. Пересчёт идёт на каждой строке, в том числе
 * на `slicer_progress` раз в ~2 с, поэтому во время нарезки шкала движется без другого вывода.
 * Прогресс монотонен всегда (откатов нет и у перепаковки). Сообщение — русское описание этапа;
 * JSON-строки (slicer_output/started/finished/progress, stage) сообщением не становятся никогда,
 * последняя английская строка уходит в [JobStage.detail].
 *
 * Планировщик/калибровка: прежние правила [ProgressRules], сообщение — последняя строка вывода
 * (кроме JSON-событий слайсера), без откатов.
 */
class ProgressTracker(
    private val kind: JobKind,
    private val clock: () -> Long = System::currentTimeMillis,
) {
    companion object {
        /** Сообщение calibration-джоба на всё время нарезки (бинарник прогресс не печатает). */
        const val CALIBRATION_MESSAGE = "Готовлю и нарезаю проекты A и B (1–2 минуты)…"

        private val PACK_FOOTPRINT_ATTEMPTS_RE = Regex("""pack_footprint:\s*(\d+)/(\d+)\s+attempts""")
        private val BEST_SO_FAR_RE = Regex("""best so far:\s*(\d+) objects""")
        private const val DETAIL_LIMIT = 300

        /** Доля полосы, до которой этап может дойти сам: конец полосы — только при смене этапа. */
        private const val MAX_FRACTION = 0.95

        private val mapper = ObjectMapper()

        /**
         * Доля этапа по времени: линейно до 80 % ожидаемого, дальше асимптотически к
         * [MAX_FRACTION] — затянувшаяся нарезка «ползёт», но не упирается в конец полосы.
         */
        internal fun timeFraction(elapsedSeconds: Double, expectedSeconds: Double): Double {
            val ratio = (elapsedSeconds / expectedSeconds.coerceAtLeast(1.0)).coerceAtLeast(0.0)
            if (ratio <= 0.8) return ratio

            return 0.8 + (MAX_FRACTION - 0.8) * (1 - exp(-(ratio - 0.8) / 0.4))
        }
    }

    // Калибровка стартует сразу с нарезки: иначе первая же строка вывода откатила бы фазу к upload.
    var phase: JobPhase = if (kind == JobKind.CALIBRATION) JobPhase.SLICING else JobPhase.UPLOAD
        private set

    var percent: Int = if (kind == JobKind.CALIBRATION) 10 else 0
        private set

    /** Последнее сообщение для пользователя (у пакера — русское описание этапа). */
    var message: String? = if (kind == JobKind.CALIBRATION) CALIBRATION_MESSAGE else null
        private set

    /** Последний j из строк "slicing sample i/j" — для правила "refining". */
    private var lastSampleTotal: Int? = null

    // ── Состояние этапов пакера ──
    private var stage: PackStage? = null
    private var stageStartedAt = 0L
    private var stageExpected: Double? = null
    private var stageObjects: Int? = null
    private var stageAttempt = 0
    /** Текущий этап открыт stage-событием (а не текстовой строкой). */
    private var stageFromEvent = false
    /** Пакер шлёт stage-события: текстовые строки больше не переключают этапы. */
    private var structured = false
    private var bandStart = 0.0
    private var bandEnd = 0.0
    /** Доля по строкам pack_footprint внутри упаковки/перепаковки. */
    private var attemptsFraction = 0.0
    private var bestSoFar: Int? = null
    /** `reason` последнего события repacking (tower_grown, support_contact, …). */
    private var repackReason: String? = null
    private var detail: String? = null
    private val visited = LinkedHashSet<String>()
    private val attempts = HashMap<PackStage, Int>()
    private var exactPercent = 0.0

    /** Текущий этап пакера (для статусной сводки); null — ещё не начался. */
    val currentStage: JobStage?
        get() = stage?.let {
            JobStage(
                id = it.id,
                startedAtMs = stageStartedAt,
                expectedSeconds = stageExpected,
                objects = stageObjects,
                attempt = stageAttempt,
                visited = visited.toList(),
                detail = detail,
            )
        }

    /**
     * Обрабатывает очередную строку вывода. Фаза/процент могут остаться прежними (строка не
     * распознана или нарушила бы монотонность).
     */
    fun onLine(line: String): ProgressEvent {
        if (line.isBlank()) return snapshot()

        val event = structuredEvent(line)
        if (kind == JobKind.PACK) {
            if (event != null) onPackEvent(event) else if (!looksLikeJson(line)) onPackText(line.trim())
            recompute()
            return snapshot()
        }

        // JSON-события слайсера (их шлют все бинарники при GOAT_LOG_FORMAT=json) — не прогресс
        // и не сообщение. Многострочный JSON калибровки сюда не попадает: в его строках нет "event".
        if (event != null) return snapshot()

        val parsed = ProgressRules.parse(kind, line, lastSampleTotal)
        if (kind == JobKind.SCHEDULE) {
            ProgressRules.SLICING_SAMPLE.find(line)?.groupValues?.get(2)?.toIntOrNull()
                ?.let { lastSampleTotal = it }
        }
        if (parsed != null) {
            val (candidatePhase, rawPercent) = parsed
            val candidatePercent = rawPercent.coerceIn(0, 99)
            if (candidatePercent >= percent) {
                phase = candidatePhase
                percent = candidatePercent
            }
        }
        // Вывод калибровки — многострочный JSON итога; показывать его построчно бессмысленно.
        message = if (kind == JobKind.CALIBRATION) CALIBRATION_MESSAGE else line

        return snapshot()
    }

    private fun snapshot() = ProgressEvent(phase, percent, message.orEmpty(), currentStage)

    private fun looksLikeJson(line: String): Boolean {
        val trimmed = line.trim()
        return trimmed.startsWith("{") && trimmed.endsWith("}")
    }

    /** JSON-объект с полем "event" или null (обычная строка, битый JSON). */
    private fun structuredEvent(line: String): JsonNode? {
        if (!looksLikeJson(line)) return null
        val node = runCatching { mapper.readTree(line.trim()) }.getOrNull() ?: return null

        return node.takeIf { it.isObject && it.path("event").isTextual }
    }

    private fun onPackEvent(event: JsonNode) {
        if (event.path("event").asText() != "stage") return // slicer_* — только повод пересчитать долю
        val next = PackStage.fromId(event.path("stage").asText()) ?: return
        val expected = event.path("expected_s").takeIf { it.isNumber }?.asDouble()?.takeIf { it > 0 }
        val objects = event.path("objects").takeIf { it.isNumber }?.asInt()?.takeIf { it > 0 }
        val wasStructured = structured
        structured = true
        if (next == PackStage.REPACKING) repackReason = event.path("reason").takeIf { it.isTextual }?.asText()

        // Тот же этап уже открыт текстовой строкой (строка пришла раньше события) — дополняем его
        // оценкой, а не перезапускаем: иначе потерялось бы прошедшее время.
        if (next == stage && !stageFromEvent && !wasStructured) {
            stageExpected = expected
            if (objects != null) stageObjects = objects
            stageFromEvent = true
            message = describe()
            return
        }
        enter(next, expected, objects, fromEvent = true)
    }

    private fun onPackText(line: String) {
        detail = line.take(DETAIL_LIMIT)

        PACK_FOOTPRINT_ATTEMPTS_RE.find(line)?.let { match ->
            val done = match.groupValues[1].toDouble()
            val total = match.groupValues[2].toDouble().coerceAtLeast(1.0)
            attemptsFraction = maxOf(attemptsFraction, (done / total).coerceIn(0.0, 1.0))
        }
        BEST_SO_FAR_RE.find(line)?.groupValues?.get(1)?.toIntOrNull()?.let { bestSoFar = it }

        if (!structured) {
            PackStageRules.stageOf(line)?.let { (next, objects) ->
                // Каждая нарезка под окно печатает свою строку — это новая попытка того же этапа.
                if (next != stage || next == PackStage.WINDOW_SLICE) {
                    enter(next, null, objects, fromEvent = false)
                }
            }
        }
        if (stage != null) message = describe()
    }

    /**
     * Начало этапа. Полоса этапа начинается не раньше текущего процента: повторные сборка/нарезка
     * после перепаковки и очередные нарезки окна продолжают с места, где шкала уже стоит, и
     * занимают половину оставшегося до конца пути (если своя полоса уже пройдена).
     */
    private fun enter(next: PackStage, expected: Double?, objects: Int?, fromEvent: Boolean) {
        stage = next
        stageStartedAt = clock()
        stageExpected = expected
        stageObjects = objects
        stageFromEvent = fromEvent
        stageAttempt = (attempts[next] ?: 0) + 1
        attempts[next] = stageAttempt
        visited.add(next.id)
        attemptsFraction = 0.0
        if (next == PackStage.PACKING || next == PackStage.REPACKING) bestSoFar = null

        bandStart = maxOf(next.bandStart.toDouble(), exactPercent)
        bandEnd = if (next.bandEnd - bandStart >= 2) next.bandEnd.toDouble() else bandStart + (99 - bandStart) / 2
        phase = next.phase
        message = describe()
    }

    private fun recompute() {
        val current = stage ?: return
        val elapsed = (clock() - stageStartedAt).coerceAtLeast(0) / 1000.0
        val expected = stageExpected ?: current.fallbackSeconds(stageObjects)
        var fraction = timeFraction(elapsed, expected)
        if (current == PackStage.PACKING || current == PackStage.REPACKING) {
            fraction = maxOf(fraction, attemptsFraction * MAX_FRACTION)
        }
        val candidate = bandStart + (bandEnd - bandStart) * fraction.coerceIn(0.0, MAX_FRACTION)
        exactPercent = maxOf(exactPercent, candidate).coerceAtMost(99.0)
        percent = maxOf(percent, exactPercent.toInt()).coerceIn(0, 99)
    }

    /** Русское описание текущего этапа для строки статуса. */
    private fun describe(): String {
        val objects = stageObjects?.let { " (${objectsText(it)})" }.orEmpty()

        return when (stage) {
            PackStage.PROBE_SLICE -> "Пробная нарезка 1 объекта: ищу место башни очистки"
            PackStage.PACKING -> "Раскладка объектов" + bestSoFar?.let { ": лучший вариант — ${objectsText(it)}" }.orEmpty()
            PackStage.ASSEMBLING -> "Сборка проекта$objects"
            PackStage.FINAL_SLICE ->
                if (PackStage.REPACKING.id in visited) "Нарезка перепакованной пластины$objects"
                else "Нарезка итоговой пластины$objects"
            PackStage.GCODE_CHECK -> "Проверка поддержек по слоям"
            PackStage.REPACKING ->
                repackTitle() + bestSoFar?.let { ", лучший вариант — ${objectsText(it)}" }.orEmpty()
            PackStage.WINDOW_SLICE ->
                "Подбор под окно времени: нарезка" + (stageObjects?.let { " ${objectsText(it)}" }.orEmpty()) +
                    (if (stageAttempt > 1) ", попытка $stageAttempt" else "")
            PackStage.WRITING -> "Запись результата"
            null -> message.orEmpty()
        }
    }

    /** Причина перепаковки из события пакера; без события (старый пакер) — башня, как раньше. */
    private fun repackTitle(): String = when (repackReason) {
        "support_contact" -> "Перепаковка: поддержки задели соседей, расширяю зону поддержек"
        "support_strict" -> "Перепаковка по строгому правилу: поддержки не заходят под соседей"
        "tower_corner_failed" -> "Перепаковка: башня в углу не прошла нарезку, возвращаю её на место"
        "tower_grown", null -> "Перепаковка: башня очистки выросла"
        else -> "Перепаковка"
    }

    private fun objectsText(count: Int): String {
        val mod100 = count % 100
        val mod10 = count % 10
        val word = when {
            mod100 in 11..14 -> "объектов"
            mod10 == 1 -> "объект"
            mod10 in 2..4 -> "объекта"
            else -> "объектов"
        }

        return "$count $word"
    }
}
