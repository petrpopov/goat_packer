package com.goatpacker.web.service.runner

import com.goatpacker.web.model.CalibrationParams
import com.goatpacker.web.model.PackDefaults
import com.goatpacker.web.model.PackParams
import com.goatpacker.web.model.ScheduleDefaults
import com.goatpacker.web.model.ScheduleParams
import com.goatpacker.web.service.NozzleCleanSpec
import com.goatpacker.web.service.StlInputs

/** Параметры джоб → аргументы командной строки Rust-бинарников (один в один, без своей логики). */
object CliCommands {
    fun pack(binary: String, params: PackParams): List<String> = buildList {
        add(binary)
        addOption("--source", StlInputs.INPUT_3MF)
        addOption("--profile", params.profilePath.toString())
        addOption("--output", JobArtifacts.PACKED_3MF)
        addOption("--work-dir", JobArtifacts.PACK_WORK_DIR)
        addOption("--clearance", params.clearance)
        // Растр не передаётся: лестница пакера (0.25, затем 0.2 мм; «Быстро» — одна 0.25) лучше
        // любого одного шага. `--resolution X` оставлял один уровень X, и веб-задачи P1S теряли
        // копию, которую даёт уровень 0.2 мм (42 вместо 43).
        addOption("--attempts", params.attempts)
        params.edgeMargin?.let { addOption("--edge-margin", it) }
        // Пусто = дефолт пакера (20 мм): передаём только заданное.
        params.towerWidth?.let { addOption("--tower-width", it) }
        // dense — все методы, grid — только ровные решётки; задачи до 2026-11-17 хранят search = dense.
        addOption("--layout", PackDefaults.cliLayout(params.layout))
        // Дефолт пакера (precise) не передаём: команда старых задач остаётся прежней.
        if (params.precision != PackDefaults.PRECISION) addOption("--precision", params.precision)
        if (params.nozzleClean.isNotBlank() && !NozzleCleanSpec.isOff(params.nozzleClean)) {
            addOption("--nozzle-clean", params.nozzleClean)
        }
        params.timeWindowHours?.let { addOption("--time-window-hours", it) }
        // --change-seconds сам включает коррекцию в CLI, поэтому без галочки его не передаём:
        // иначе значение, оставшееся в форме, молча включило бы коррекцию.
        if (params.amsCorrection) {
            add("--ams-correction")
            params.changeSeconds?.let { addOption("--change-seconds", it) }
        }
        params.timeMarginPct?.let { addOption("--time-margin-pct", it) }
        // Пусто = без ограничений (поведение до флага), поэтому передаём только заданное.
        params.supportOverlapMm?.let { addOption("--support-overlap-mm", it) }
        // Стол не режется: проверка нарезкой убрана из webapp (2026-11-17). Окно печати пакер режет сам.
        // Слайсер STL-задачи: пакер переключает профиль на него и берёт его GUI целевым.
        params.slicer?.let { addOption("--slicer", it) }
        // Пробы одного класса поворота — самая долгая часть холодного запуска; кэш держит воркер, не раннер.
        params.cacheDir?.let { addOption("--cache-dir", it) }
    }

    fun schedule(binary: String, params: ScheduleParams): List<String> = buildList {
        add(binary)
        // ВАЖНО: аргумент после --printer — одна строка с запятыми.
        addOption("--printer", printerSpec(params))
        if (params.amsCorrection) add("--ams-correction")
        addOption("--output-schedule", JobArtifacts.SCHEDULE_CSV)
        addOption("--output-report", JobArtifacts.SCHEDULE_REPORT)
        addOption("--turnaround-min", params.turnaroundMin)
        addOption("--day-start", params.dayStart)
        addOption("--night-start", params.nightStart)
        addOption("--horizon-end", params.horizonEnd)
        if (params.benchMode.equals(ScheduleDefaults.BENCH_MODE_FULL, ignoreCase = true)) {
            addOption("--bench-mode", ScheduleDefaults.BENCH_MODE_FULL)
        }
    }

    fun calibration(binary: String, params: CalibrationParams): List<String> = listOf(
        binary,
        "--template", StlInputs.INPUT_3MF,
        "--profile", params.profilePath.toString(),
        "--variant", params.variant,
        "--out-dir", JobArtifacts.CALIBRATION_OUT_DIR,
    )

    private fun printerSpec(params: ScheduleParams): String {
        // change-s= сам включает коррекцию — только вместе с --ams-correction.
        val changeSeconds = params.changeSeconds?.takeIf { params.amsCorrection }?.let { "change-s=$it," }.orEmpty()
        return "profile=${params.profilePath},source=${StlInputs.INPUT_3MF},prep-min=${params.prepMin},${changeSeconds}plates-3mf=${JobArtifacts.PLATES_3MF}"
    }

    private fun MutableList<String>.addOption(name: String, value: Any) {
        add(name)
        add(value.toString())
    }
}
