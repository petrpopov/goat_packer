package com.goatpacker.web.service.runner

import com.goatpacker.web.model.CalibrationParams
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
        addOption("--resolution", params.resolution)
        addOption("--attempts", params.attempts)
        addOption("--candidates", params.candidates)
        addOption("--seed", params.seed)
        params.edgeMargin?.let { addOption("--edge-margin", it) }
        addOption("--layout", params.layout)
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
        if (!params.finalSlice) add("--no-final-slice")
        params.finishState?.let { addOption("--finish-state", it.toString()) }
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
