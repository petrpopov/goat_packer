package com.goatpacker.web.service.runner

import com.fasterxml.jackson.databind.JsonNode
import com.fasterxml.jackson.databind.ObjectMapper
import com.goatpacker.web.model.CalibrationJobResult
import com.goatpacker.web.model.CalibrationParams
import com.goatpacker.web.model.PackJobResult
import com.goatpacker.web.model.PackParams
import com.goatpacker.web.model.PackTimeResult
import com.goatpacker.web.model.PlateRecord
import com.goatpacker.web.model.ScheduleJobRecord
import com.goatpacker.web.model.ScheduleJobResult
import com.goatpacker.web.util.ClockFormat
import org.slf4j.LoggerFactory
import org.springframework.stereotype.Component
import java.nio.charset.StandardCharsets
import java.nio.file.Files
import java.nio.file.Path

/** Разбор отчётов Rust-бинарников (verification.json, calibration.json, report.json) в результаты джоб. */
@Component
class JobReportParser(private val mapper: ObjectMapper) {
    private val log = LoggerFactory.getLogger(javaClass)

    /** Результат упаковки по `packed.verification.json`; null — в отчёте нет `object_count` (джоб считается упавшим). */
    fun parsePack(dir: Path, params: PackParams): PackJobResult? {
        val report = dir.resolve(JobArtifacts.PACKED_REPORT).takeIf { Files.exists(it) }?.let { mapper.readTree(it.toFile()) }
        val objectCount = report?.get("object_count")?.asInt(-1) ?: -1
        if (objectCount < 0) return null

        return PackJobResult(
            objectCount = objectCount,
            profileId = params.profileId,
            clearanceMm = params.clearance,
            time = report?.get("time")?.let(::parsePackTime),
            supportsDetected = report?.get("supports_detected")?.takeIf { it.isBoolean }?.asBoolean(),
            supportOverlapMm = report?.get("support_overlap_mm")?.takeIf { it.isNumber }?.asDouble(),
            // Старый пакер поля не пишет — он всегда резал стол целиком.
            finalSliced = report?.get("final_sliced")?.takeIf { it.isBoolean }?.asBoolean() ?: true,
        )
    }

    /** calibration.json → [CalibrationJobResult]; инструкция читается целиком (единицы КБ). Бросает при битом отчёте. */
    fun parseCalibration(dir: Path, params: CalibrationParams): CalibrationJobResult {
        val node = mapper.readTree(dir.resolve(JobArtifacts.CALIBRATION_REPORT).toFile())

        fun number(name: String): Double = node.get(name)?.takeIf { it.isNumber }?.asDouble() ?: error("поле $name отсутствует")
        return CalibrationJobResult(
            profileId = params.profileId,
            variant = node.path("variant").asText(params.variant),
            changesExpected = number("changes_expected").toInt(),
            changesSliced = number("changes_sliced").toInt(),
            estimateASeconds = number("estimate_a_s"),
            estimateBSeconds = number("estimate_b_s"),
            slicerChangeSeconds = number("slicer_change_s"),
            instructions = Files.readString(dir.resolve(JobArtifacts.CALIBRATION_INSTRUCTIONS), StandardCharsets.UTF_8)
                .take(MAX_INSTRUCTIONS_CHARS),
        )
    }

    /** report.json → printers[0].variable_batch_schedule → [ScheduleJobResult] (plates считаются тут); null — отчёт непригоден. */
    fun parseSchedule(dir: Path, jobId: String): ScheduleJobResult? {
        val reportPath = dir.resolve(JobArtifacts.SCHEDULE_REPORT)
        if (!Files.exists(reportPath)) return null

        val root = try {
            mapper.readTree(reportPath.toFile())
        } catch (ex: Exception) {
            log.error("report.json джобы {} не читается: {}", jobId, ex.message)
            return null
        }
        val printerAnalysis = root.get("printers")?.get(0) ?: return null
        val schedule = printerAnalysis.get("variable_batch_schedule") ?: return null
        val jobs = schedule.get("jobs")?.takeIf { it.isArray }?.map(::parseScheduleJob) ?: return null
        if (jobs.isEmpty()) return null

        // Пластины: уникальные batch_size в порядке появления, подпись — по первой работе батча.
        val firstJobOfBatch = linkedMapOf<Int, ScheduleJobRecord>()
        for (job in jobs) firstJobOfBatch.putIfAbsent(job.batchSize, job)

        return ScheduleJobResult(
            printer = printerAnalysis.path("printer").asText(""),
            totalObjects = schedule.path("total_objects").asInt(0),
            totalFilamentGrams = schedule.path("total_filament_grams").asDouble(0.0),
            completionClock = ClockFormat.format(schedule.path("completion_seconds").asLong(0)),
            jobs = jobs,
            plates = firstJobOfBatch.map { (batchSize, first) ->
                PlateRecord(label = "$batchSize pcs @ ${ClockFormat.format(first.startSeconds)}", objectCount = batchSize)
            },
        )
    }

    private fun parseScheduleJob(node: JsonNode) = ScheduleJobRecord(
        sequence = node.path("sequence").asInt(),
        batchSize = node.path("batch_size").asInt(),
        startSeconds = node.path("start_seconds").asLong(),
        endSeconds = node.path("end_seconds").asLong(),
        slicerSeconds = node.path("slicer_seconds").asLong(),
        preparationSeconds = node.path("preparation_seconds").asLong(),
        filamentGrams = node.path("filament_grams").asDouble(),
        cumulativeObjects = node.path("cumulative_objects").asInt(),
        overnight = node.path("overnight").asBoolean(),
    )

    /**
     * Блок `time` отчёта упаковщика → [PackTimeResult]. null/битый блок = время не измерено
     * (стол при этом валиден), поэтому ошибкой джобы это не считается. Полный стол — наибольший
     * count среди замеров: с окном это второй замер (после пробы на 1 объект), без окна — сам стол.
     */
    private fun parsePackTime(node: JsonNode): PackTimeResult? {
        if (!node.isObject) return null
        val corrected = node.get("corrected_s")?.takeIf { it.isNumber }?.asDouble() ?: return null
        val chosen = node.get("chosen_count")?.takeIf { it.canConvertToInt() }?.asInt() ?: return null
        val fullCount = node.path("samples").mapNotNull { sample -> sample.get("count")?.takeIf { it.isInt }?.asInt() }
            .maxOrNull() ?: chosen
        return PackTimeResult(
            windowHours = node.get("window_hours")?.takeIf { it.isNumber }?.asDouble(),
            marginPct = node.path("margin_pct").asDouble(0.0),
            realChangeSeconds = node.get("real_change_s")?.takeIf { it.isNumber }?.asDouble(),
            chosenCount = chosen,
            fullCount = maxOf(fullCount, chosen),
            correctedSeconds = corrected,
        )
    }

    private companion object {
        const val MAX_INSTRUCTIONS_CHARS = 64 * 1024
    }
}
