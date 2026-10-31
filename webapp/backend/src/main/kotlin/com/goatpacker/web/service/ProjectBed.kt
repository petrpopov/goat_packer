package com.goatpacker.web.service

import com.fasterxml.jackson.databind.JsonNode

/**
 * Стол из project_settings.config 3MF неизвестного принтера: области в координатах стола, мм.
 * [printableArea] — [x0, y0, x1, y1]; [exclusionRectangles] — прямоугольники [x0, y0, x1, y1];
 * [printableHeight] — null, если в проекте нет (или не число).
 */
data class ProjectBed(
    val printableArea: List<Double>,
    val exclusionRectangles: List<List<Double>>,
    val printableHeight: Double?,
)

/**
 * Разбор точек Orca/Bambu — порт orca_presets::{parse_points, rect_from_points, exclusion_rects}
 * (правила обязаны совпадать с каталогом, иначе один и тот же стол дал бы разную геометрию):
 * список строк ["0x0","256x0",…] или одна строка "0x0,256x0,…".
 */
object OrcaBed {
    fun points(value: JsonNode?): List<Pair<Double, Double>>? {
        val items = when {
            value == null -> return null
            value.isArray -> value.filter { it.isTextual }.map { it.asText() }
            value.isTextual -> value.asText().split(',')
            else -> return null
        }
        return items.map { item ->
            val (x, y) = item.trim().split('x', limit = 2).takeIf { it.size == 2 } ?: return null
            val px = x.trim().toDoubleOrNull()?.takeIf { it.isFinite() } ?: return null
            val py = y.trim().toDoubleOrNull()?.takeIf { it.isFinite() } ?: return null
            px to py
        }
    }

    /** [x0,y0,x1,y1], только если точки — ровно осевой прямоугольник; круглый/фигурный стол → null. */
    fun rect(points: List<Pair<Double, Double>>): List<Double>? {
        val xs = distinct(points.map { it.first }); val ys = distinct(points.map { it.second })
        return if (points.size == 4 && xs.size == 2 && ys.size == 2 && xs[1] > xs[0] && ys[1] > ys[0]) listOf(xs[0], ys[0], xs[1], ys[1]) else null
    }

    /** bed_exclude_area: многоугольник → его габаритный прямоугольник (с запасом), вырожденный ("0x0") — нет зоны. */
    fun exclusions(value: JsonNode?): List<List<Double>> {
        val points = points(value) ?: return emptyList()
        if (points.size < 3) return emptyList()
        val minX = points.minOf { it.first }; val maxX = points.maxOf { it.first }
        val minY = points.minOf { it.second }; val maxY = points.maxOf { it.second }
        if (maxX - minX < 1e-6 || maxY - minY < 1e-6) return emptyList()
        return listOf(listOf(minX, minY, maxX, maxY))
    }

    /** null — в проекте нет printable_area или стол не прямоугольный (такой пакер не опишет). */
    fun fromSettings(settings: JsonNode): ProjectBed? {
        val area = points(settings.get("printable_area"))?.let(::rect) ?: return null
        val height = settings.get("printable_height")?.let { if (it.isArray) it.firstOrNull() else it }
            ?.let { if (it.isNumber) it.asDouble() else it.asText().trim().toDoubleOrNull() }
            ?.takeIf { it.isFinite() && it > 0 }
        return ProjectBed(area, exclusions(settings.get("bed_exclude_area")), height)
    }

    private fun distinct(values: List<Double>): List<Double> =
        values.sorted().fold(mutableListOf()) { acc, v -> if (acc.isEmpty() || v - acc.last() >= 1e-6) acc.add(v); acc }
}
