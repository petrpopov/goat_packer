package com.goatpacker.web.service

import com.goatpacker.web.job.ScheduleJobRecord
import org.apache.poi.ss.usermodel.FillPatternType
import org.apache.poi.ss.usermodel.IndexedColors
import org.apache.poi.ss.usermodel.WorkbookFactory
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
    companion object {
        private val log = LoggerFactory.getLogger(XlsxWriter::class.java)

        private val HEADERS = listOf(
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
        private const val SHEET_NAME = "Расписание"
    }

    fun write(target: Path, printer: String, jobs: List<ScheduleJobRecord>) {
        XSSFWorkbook().use { workbook ->
            val sheet = workbook.createSheet(SHEET_NAME)

            val headerStyle = workbook.createCellStyle().apply {
                val font = workbook.createFont()
                font.bold = true
                setFont(font)
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

            jobs.forEachIndexed { rowIndex, job ->
                val row = sheet.createRow(rowIndex + 1)
                row.createCell(0).setCellValue(printer)
                row.createCell(1).setCellValue(job.sequence.toDouble())
                row.createCell(2).setCellValue(job.batchSize.toDouble())
                row.createCell(3).setCellValue(job.startSeconds.toDouble())
                row.createCell(4).setCellValue(job.endSeconds.toDouble())
                row.createCell(5).setCellValue(ClockFormat.format(job.startSeconds))
                row.createCell(6).setCellValue(ClockFormat.format(job.endSeconds))
                row.createCell(7).setCellValue(job.slicerSeconds.toDouble())
                row.createCell(8).setCellValue(job.preparationSeconds.toDouble())
                row.createCell(9).setCellValue(job.filamentGrams)
                row.createCell(10).setCellValue(job.cumulativeObjects.toDouble())
                row.createCell(11).setCellValue(if (job.overnight) "true" else "false")
            }

            for (column in HEADERS.indices) {
                sheet.autoSizeColumn(column)
            }
            sheet.createFreezePane(0, 1)

            FileOutputStream(target.toFile()).use { workbook.write(it) }
        }
        log.info("Расписание записано в {}: работ — {}", target, jobs.size)
    }

    /** Контрольный прогон: файл открывается WorkbookFactory и не пуст. */
    fun canOpen(target: Path): Boolean = try {
        WorkbookFactory.create(target.toFile()).use { workbook -> workbook.numberOfSheets > 0 }
    } catch (ex: Exception) {
        log.warn("XLSX {} не открывается: {}", target, ex.message)
        false
    }
}
