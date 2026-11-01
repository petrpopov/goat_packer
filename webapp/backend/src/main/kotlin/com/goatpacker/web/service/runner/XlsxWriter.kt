package com.goatpacker.web.service.runner

import com.goatpacker.web.model.ScheduleJobRecord
import com.goatpacker.web.util.ClockFormat
import org.apache.poi.ss.usermodel.FillPatternType
import org.apache.poi.ss.usermodel.IndexedColors
import org.apache.poi.xssf.usermodel.XSSFWorkbook
import org.slf4j.LoggerFactory
import org.springframework.stereotype.Component
import java.io.FileOutputStream
import java.nio.file.Path

/**
 * Запись расписания в XLSX (Apache POI): лист «Расписание», жирная шапка с заливкой, закрепление
 * первой строки, автоподбор ширины колонок. Колонки — ровно как в schedule.csv.
 */
@Component
class XlsxWriter {
    private val log = LoggerFactory.getLogger(javaClass)

    fun write(target: Path, printer: String, jobs: List<ScheduleJobRecord>) {
        XSSFWorkbook().use { workbook ->
            val sheet = workbook.createSheet(SHEET_NAME)
            val headerStyle = workbook.createCellStyle().apply {
                setFont(workbook.createFont().apply { bold = true })
                fillForegroundColor = IndexedColors.GREY_25_PERCENT.index
                fillPattern = FillPatternType.SOLID_FOREGROUND
            }

            sheet.createRow(0).apply {
                HEADERS.forEachIndexed { index, title ->
                    createCell(index).apply {
                        setCellValue(title)
                        cellStyle = headerStyle
                    }
                }
            }
            jobs.forEachIndexed { index, job ->
                val row = sheet.createRow(index + 1)
                rowValues(printer, job).forEachIndexed { column, value ->
                    when (value) {
                        is Number -> row.createCell(column).setCellValue(value.toDouble())
                        else -> row.createCell(column).setCellValue(value.toString())
                    }
                }
            }
            for (column in HEADERS.indices) sheet.autoSizeColumn(column)
            sheet.createFreezePane(0, 1)

            FileOutputStream(target.toFile()).use { workbook.write(it) }
        }
        log.info("Расписание записано в {}: работ — {}", target, jobs.size)
    }

    /** Значения строки в порядке [HEADERS]. */
    private fun rowValues(printer: String, job: ScheduleJobRecord): List<Any> = listOf(
        printer,
        job.sequence,
        job.batchSize,
        job.startSeconds,
        job.endSeconds,
        ClockFormat.format(job.startSeconds),
        ClockFormat.format(job.endSeconds),
        job.slicerSeconds,
        job.preparationSeconds,
        job.filamentGrams,
        job.cumulativeObjects,
        job.overnight,
    )

    private companion object {
        const val SHEET_NAME = "Расписание"

        val HEADERS = listOf(
            "printer",
            "sequence",
            "batch_size",
            "start_seconds",
            "end_seconds",
            "start_clock",
            "end_clock",
            "slicer_seconds",
            "preparation_seconds",
            "filament_grams",
            "cumulative_objects",
            "overnight",
        )
    }
}
