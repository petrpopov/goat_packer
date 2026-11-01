package com.goatpacker.web.service.runner

import com.goatpacker.web.model.JobPhase

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
    PACKING("packing", JobPhase.PACKING, 15, 24, 60.0),
    SAMPLE_SLICE("sample_slice", JobPhase.SLICING, 24, 30, 30.0),
    ASSEMBLING("assembling", JobPhase.ASSEMBLING, 30, 33, 10.0),
    FINAL_SLICE("final_slice", JobPhase.SLICING, 33, 78, 300.0),
    GCODE_CHECK("gcode_check", JobPhase.SLICING, 78, 82, 30.0),
    REPACKING("repacking", JobPhase.REPACKING, 82, 86, 60.0),
    WINDOW_SLICE("window_slice", JobPhase.SLICING, 86, 95, 300.0),
    WRITING("writing", JobPhase.WRITING, 95, 99, 10.0);

    val slicing: Boolean
        get() = this == PROBE_SLICE || this == SAMPLE_SLICE || this == FINAL_SLICE || this == WINDOW_SLICE

    /** Ожидаемая длительность при отсутствии `expected_s`: для нарезок — по формуле из дизайна. */
    fun fallbackSeconds(objects: Int?): Double =
        if ((this == FINAL_SLICE || this == WINDOW_SLICE) && objects != null && objects > 0) {
            PROBE_SECONDS * (1 + SECONDS_GROWTH_PER_OBJECT * (objects - 1))
        } else {
            defaultSeconds
        }

    companion object {
        /** Типичная пробная нарезка одного объекта на сервере, с (из замеров дизайна). */
        private const val PROBE_SECONDS = 22.0

        /** Каждый следующий объект стола удлиняет нарезку на эту долю пробной (из тех же замеров). */
        private const val SECONDS_GROWTH_PER_OBJECT = 0.36

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
