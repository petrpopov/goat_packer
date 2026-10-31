package com.goatpacker.web.service

import com.fasterxml.jackson.databind.ObjectMapper
import com.fasterxml.jackson.databind.node.ObjectNode
import com.goatpacker.web.dto.CustomBed
import com.goatpacker.web.dto.ProfileDto

/**
 * Разовый профиль для своего стола: тот же формат, что json-профили каталога (PrinterProfile в Rust),
 * но собран из размеров пользователя. Чего мы о незнакомом принтере не знаем, то выключено:
 * линий прочистки нет (их заменяет [CustomBed.margin]), чистки сопла и AMS-коррекции нет.
 */
object CustomProfile {
    const val ID = "custom"
    const val NAME = "Custom Printer"

    data class Built(val dto: ProfileDto, val json: String)

    /**
     * [projectBed] — стол из 3MF неизвестного принтера. Он берётся (с его началом координат и
     * зонами исключения), только если пользователь подтвердил его размеры без правки: иначе
     * введённые цифры важнее, и стол строится от (0, 0) без исключений.
     */
    fun build(mapper: ObjectMapper, bed: CustomBed, projectBed: ProjectBed?, slicerPath: String): Built {
        val confirmed = projectBed?.takeIf { p ->
            Math.abs(p.printableArea[2] - p.printableArea[0] - bed.width) < 0.01 && Math.abs(p.printableArea[3] - p.printableArea[1] - bed.depth) < 0.01
        }
        val area = confirmed?.printableArea ?: listOf(0.0, 0.0, bed.width, bed.depth)
        val exclusions = confirmed?.exclusionRectangles ?: emptyList()
        val node: ObjectNode = mapper.createObjectNode().apply {
            put("id", ID); put("printer_name", NAME); put("download_slug", ID)
            // Orca: её CLI грузит и Bambu-проекты; своего слайсера у незнакомого принтера нет.
            put("slicer_kind", "orca"); put("slicer_path", slicerPath)
            put("blank_start_gcode_for_cli", false)
            // Башня незнакомого принтера в пробе не проверяется: её размер мы не калибровали.
            put("no_check_for_final_tower_probe", true)
            putArray("printable_area").apply { area.forEach { add(it) } }
            put("printable_height", bed.height)
            put("edge_margin_mm", bed.margin)
            putArray("exclusion_rectangles").apply { exclusions.forEach { r -> addArray().apply { r.forEach { add(it) } } } }
            putArray("purge_lines")
            put("tower_marker_style", "wipe_tower"); put("tower_line_width_mm", 0.5)
        }
        val dto = ProfileDto(
            id = ID, name = NAME, vendor = "", slicerKind = "orca",
            bedWidthMm = area[2] - area[0], bedDepthMm = area[3] - area[1], edgeMarginMm = bed.margin,
            supportsNozzleClean = false, filamentChangeSeconds = null,
            printableArea = area, exclusionRectangles = exclusions,
        )
        return Built(dto, mapper.writeValueAsString(node))
    }
}
