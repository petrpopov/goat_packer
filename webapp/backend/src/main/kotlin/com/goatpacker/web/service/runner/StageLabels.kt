package com.goatpacker.web.service.runner

import com.fasterxml.jackson.databind.JsonNode
import java.math.BigDecimal

/**
 * Русские подписи шагов внутри этапа из `detail` stage-события пакера (`progress::StageDetail`):
 * живая подпись идущего шага и короткий итог этапа для «Инфо». Пакер шлёт подряд несколько
 * событий одного этапа (прогоны пробы по поворотам, варианты зазора и уровни сетки раскладки) —
 * для пользователя это один этап, а шаги видны только в подписи.
 */
object StageLabels {
    /** Подпись текущего шага; null — у этапа нет шагов или поля незнакомые. */
    fun label(stage: PackStage, detail: JsonNode): String? = when (stage) {
        PackStage.PROBE_SLICE, PackStage.MODELLING -> probeLabel(detail)
        PackStage.PACKING -> packingLabel(detail)
        else -> null
    }

    private fun probeLabel(detail: JsonNode): String? {
        if (detail.path("purpose").asText() == "time") return "оценка времени печати"
        val rotation = detail.path("rotation_deg").takeIf { it.isNumber }?.let { degrees(it.asDouble()) } ?: return null
        if (detail.path("cached").asBoolean(false)) return "поворот $rotation · из кэша"
        val run = detail.path("run").takeIf { it.isInt }?.asInt() ?: return "поворот $rotation"
        val maxRuns = detail.path("max_runs").takeIf { it.isInt }?.asInt()

        return "поворот $rotation · прогон $run" + maxRuns?.let { " (до $it)" }.orEmpty()
    }

    private fun packingLabel(detail: JsonNode): String? {
        val parts = ArrayList<String>()
        val variants = detail.path("variants").asInt(0)
        if (variants > 1) parts += "вариант зазора ${detail.path("variant").asInt(1)} из $variants"
        detail.path("resolution_mm").takeIf { it.isNumber }?.let {
            val grid = "сетка ${millimetres(it.asDouble())} мм"
            parts += if (detail.path("phase").asText() == "refine") "уточнение · $grid" else grid
        }

        return parts.takeIf { it.isNotEmpty() }?.joinToString(" · ")
    }

    /** 180.0 → «180°», 22.5 → «22,5°». */
    fun degrees(value: Double): String = "${decimal(value)}°"

    /** 0.25 → «0,25». */
    fun millimetres(value: Double): String = decimal(value)

    private fun decimal(value: Double): String =
        BigDecimal.valueOf(value).stripTrailingZeros().toPlainString().replace('.', ',')

    /** Склонение: 1 прогон, 2 прогона, 5 прогонов, 11 прогонов. */
    fun plural(count: Int, one: String, few: String, many: String): String {
        val mod100 = count % 100
        val mod10 = count % 10
        val word = when {
            mod100 in 11..14 -> many
            mod10 == 1 -> one
            mod10 in 2..4 -> few
            else -> many
        }

        return "$count $word"
    }
}

/**
 * Итог шагов одного захода в этап для строки «Инфо»: у пробы — прогоны по поворотам
 * («0°: 5 прогонов · 180°: из кэша»), у раскладки — варианты зазора и пройденные сетки
 * («4 варианта зазора · сетка 0,25 → 0,2 мм»). Копится по мере событий, сбрасывается новым заходом.
 */
class StageNotes {
    private val runs = LinkedHashMap<String, Int>()
    private val cached = HashSet<String>()
    private var variants = 0
    private val resolutions = LinkedHashSet<String>()

    fun add(stage: PackStage, detail: JsonNode) {
        when (stage) {
            PackStage.PROBE_SLICE, PackStage.MODELLING -> addProbe(detail)
            PackStage.PACKING -> {
                variants = maxOf(variants, detail.path("variants").asInt(0))
                detail.path("resolution_mm").takeIf { it.isNumber }?.let { resolutions += StageLabels.millimetres(it.asDouble()) }
            }
            else -> Unit
        }
    }

    private fun addProbe(detail: JsonNode) {
        if (detail.path("purpose").asText() == "time") return
        val rotation = detail.path("rotation_deg").takeIf { it.isNumber }?.let { StageLabels.degrees(it.asDouble()) } ?: return
        if (detail.path("cached").asBoolean(false)) cached += rotation
        runs[rotation] = maxOf(runs[rotation] ?: 0, detail.path("run").asInt(0))
    }

    fun note(stage: PackStage): String? = when (stage) {
        // Модель поддержек слайсер не вызывает: прогонов нет, только повороты.
        PackStage.MODELLING -> runs.keys.takeIf { it.isNotEmpty() }?.joinToString(", ", prefix = "повороты ")
        PackStage.PROBE_SLICE ->
            runs.entries.takeIf { it.isNotEmpty() }?.joinToString(" · ") { (rotation, count) ->
                "$rotation: " + if (rotation in cached) "из кэша" else StageLabels.plural(count, "прогон", "прогона", "прогонов")
            }
        PackStage.PACKING -> {
            val parts = ArrayList<String>()
            if (variants > 1) parts += StageLabels.plural(variants, "вариант", "варианта", "вариантов") + " зазора"
            if (resolutions.isNotEmpty()) parts += "сетка ${resolutions.joinToString(" → ")} мм"
            parts.takeIf { it.isNotEmpty() }?.joinToString(" · ")
        }
        else -> null
    }
}
