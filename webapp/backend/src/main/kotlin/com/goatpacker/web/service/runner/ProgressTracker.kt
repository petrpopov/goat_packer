package com.goatpacker.web.service.runner

import com.fasterxml.jackson.databind.JsonNode
import com.fasterxml.jackson.databind.ObjectMapper
import com.goatpacker.web.model.JobKind
import com.goatpacker.web.model.JobPhase
import com.goatpacker.web.model.JobStage
import com.goatpacker.web.model.StageSpan
import kotlin.math.exp

/** Результат разбора одной строки вывода бинарника: фаза, процент, сообщение, этап пакера. */
data class ProgressEvent(
    val phase: JobPhase,
    val progressPercent: Int,
    val phaseMessage: String,
    /** Текущий этап упаковки (только pack-джобы); null — этап ещё не начался или не пакер. */
    val stage: JobStage? = null,
    /** Последнее layout-событие пакера (сырой JSON) — для сцены стола; null — раскладки ещё нет. */
    val layout: String? = null,
)

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
    /**
     * Сообщение до первого этапа пакера (конвертация STL): фаза — проба, процент — начало её
     * полосы. Строки до первого этапа сообщение пакера не меняют, поэтому оно держится само.
     */
    initialMessage: String? = null,
    private val clock: () -> Long = System::currentTimeMillis,
) {
    companion object {
        /** Сообщение calibration-джоба на всё время нарезки (бинарник прогресс не печатает). */
        const val CALIBRATION_MESSAGE = "Готовлю и нарезаю проекты A и B (1–2 минуты)…"

        /** Процент, с которого стартует калибровка: нарезка начинается сразу. */
        const val CALIBRATION_START_PERCENT = 10

        /** Выше процент не поднимается, пока процесс жив: 100 ставит JobRunner по завершении. */
        private const val MAX_RUNNING_PERCENT = 99

        private val PACK_FOOTPRINT_ATTEMPTS_RE = Regex("""pack_footprint:\s*(\d+)/(\d+)\s+attempts""")
        private val BEST_SO_FAR_RE = Regex("""best so far:\s*(\d+) objects""")
        private const val DETAIL_LIMIT = 300

        /** ~85 символов на заход: 30 заходов + detail укладываются в 4000 символов stage_json. */
        private const val MAX_HISTORY = 30

        /** Доля полосы, до которой этап может дойти сам: конец полосы — только при смене этапа. */
        private const val MAX_FRACTION = 0.95

        /** До этой доли ожидаемого времени этап идёт линейно, дальше — асимптотически к [MAX_FRACTION]. */
        private const val LINEAR_FRACTION = 0.8

        /** Скорость затухания хвоста: чем больше, тем медленнее шкала «ползёт» после ожидаемого времени. */
        private const val TAIL_DECAY = 0.4

        /** Полоса короче этого числа процентов считается слишком узкой и расширяется (повторные этапы). */
        private const val MIN_BAND_WIDTH = 2

        private val mapper = ObjectMapper()

        /**
         * Доля этапа по времени: линейно до 80 % ожидаемого, дальше асимптотически к
         * [MAX_FRACTION] — затянувшаяся нарезка «ползёт», но не упирается в конец полосы.
         */
        internal fun timeFraction(elapsedSeconds: Double, expectedSeconds: Double): Double {
            val ratio = (elapsedSeconds / expectedSeconds.coerceAtLeast(1.0)).coerceAtLeast(0.0)
            if (ratio <= LINEAR_FRACTION) return ratio

            return LINEAR_FRACTION + (MAX_FRACTION - LINEAR_FRACTION) * (1 - exp(-(ratio - LINEAR_FRACTION) / TAIL_DECAY))
        }
    }

    // Калибровка стартует сразу с нарезки: иначе первая же строка вывода откатила бы фазу к upload.
    var phase: JobPhase = when {
        kind == JobKind.CALIBRATION -> JobPhase.SLICING
        initialMessage != null -> PackStage.PROBE_SLICE.phase
        else -> JobPhase.UPLOAD
    }
        private set

    var percent: Int = when {
        kind == JobKind.CALIBRATION -> CALIBRATION_START_PERCENT
        initialMessage != null -> PackStage.PROBE_SLICE.bandStart
        else -> 0
    }
        private set

    /** Последнее сообщение для пользователя (у пакера — русское описание этапа). */
    var message: String? = if (kind == JobKind.CALIBRATION) CALIBRATION_MESSAGE else initialMessage
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
    private val history = ArrayList<StageSpan>()
    private val attempts = HashMap<PackStage, Int>()
    private var exactPercent = 0.0

    /** Сырая строка последнего layout-события: отдаётся как есть, разбирать её — дело сцены. */
    private var layoutJson: String? = null

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
                history = historySnapshot(),
            )
        }

    /**
     * История заходов для stage_json (лимит 4000 символов у столбца: усечённый JSON не разобрать, поэтому
     * при переполнении отбрасываются самые старые заходы, а не хвост строки). Текущий заход всегда остаётся.
     */
    private fun historySnapshot(): List<StageSpan> = history.takeLast(MAX_HISTORY)

    /**
     * Обрабатывает очередную строку вывода. Фаза/процент могут остаться прежними (строка не
     * распознана или нарушила бы монотонность).
     */
    fun onLine(line: String): ProgressEvent {
        if (line.isBlank()) return snapshot()

        val event = structuredEvent(line)
        if (kind == JobKind.PACK) {
            // Раскладка — не этап и не сообщение: только запоминается (живёт до следующей).
            if (event != null && event.path("event").asText() == "layout") {
                layoutJson = line.trim()
                return snapshot()
            }
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
            val candidatePercent = rawPercent.coerceIn(0, MAX_RUNNING_PERCENT)
            if (candidatePercent >= percent) {
                phase = candidatePhase
                percent = candidatePercent
            }
        }
        // Вывод калибровки — многострочный JSON итога; показывать его построчно бессмысленно.
        message = if (kind == JobKind.CALIBRATION) CALIBRATION_MESSAGE else line

        return snapshot()
    }

    private fun snapshot() = ProgressEvent(phase, percent, message.orEmpty(), currentStage, layoutJson)

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
        val now = clock()
        history.lastOrNull()?.takeIf { it.endedAtMs == null }?.let { history[history.lastIndex] = it.copy(endedAtMs = now) }
        stage = next
        stageStartedAt = now
        stageExpected = expected
        stageObjects = objects
        stageFromEvent = fromEvent
        stageAttempt = (attempts[next] ?: 0) + 1
        attempts[next] = stageAttempt
        visited.add(next.id)
        history.add(StageSpan(next.id, stageAttempt, now))
        attemptsFraction = 0.0
        if (next == PackStage.PACKING || next == PackStage.REPACKING) bestSoFar = null

        bandStart = maxOf(next.bandStart.toDouble(), exactPercent)
        bandEnd = if (next.bandEnd - bandStart >= MIN_BAND_WIDTH) next.bandEnd.toDouble() else bandStart + (MAX_RUNNING_PERCENT - bandStart) / 2
        phase = next.phase
        message = describe()
    }

    private fun recompute() {
        val current = stage ?: return
        val elapsed = (clock() - stageStartedAt).coerceAtLeast(0) / MILLIS_PER_SECOND
        val expected = stageExpected ?: current.fallbackSeconds(stageObjects)
        var fraction = timeFraction(elapsed, expected)
        if (current == PackStage.PACKING || current == PackStage.REPACKING) {
            fraction = maxOf(fraction, attemptsFraction * MAX_FRACTION)
        }
        val candidate = bandStart + (bandEnd - bandStart) * fraction.coerceIn(0.0, MAX_FRACTION)
        exactPercent = maxOf(exactPercent, candidate).coerceAtMost(MAX_RUNNING_PERCENT.toDouble())
        percent = maxOf(percent, exactPercent.toInt()).coerceIn(0, MAX_RUNNING_PERCENT)
    }

    /** Русское описание текущего этапа для строки статуса. */
    private fun describe(): String {
        val objects = stageObjects?.let { " (${objectsText(it)})" }.orEmpty()

        return when (stage) {
            PackStage.PROBE_SLICE -> "Пробная нарезка 1 объекта: ищу место башни очистки"
            PackStage.PACKING -> "Раскладка объектов" + bestSoFar?.let { ": лучший вариант — ${objectsText(it)}" }.orEmpty()
            PackStage.SAMPLE_SLICE -> "Пробная нарезка$objects: снимаю реальные поддержки"
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
        "support_nudge" -> "Поддержки задели соседей: раздвигаю только эти объекты"
        "support_contact" -> "Перепаковка: поддержки задели соседей, расширяю зону поддержек"
        "support_strict" -> "Перепаковка по строгому правилу: поддержки не заходят под соседей"
        "support_off_plate" -> "Перепаковка: поддержки вышли за край стола, расширяю зону поддержек"
        "tower_corner_failed" -> "Перепаковка: башня в углу не прошла нарезку, возвращаю её на место"
        "tower_grown", null -> "Перепаковка: башня очистки выросла"
        else -> "Перепаковка"
    }

    /** Число с правильным склонением: «1 объект», «2 объекта», «5 объектов», «11 объектов». */
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

private const val MILLIS_PER_SECOND = 1000.0
