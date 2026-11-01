package com.goatpacker.web.service

import com.fasterxml.jackson.databind.ObjectMapper
import com.fasterxml.jackson.databind.node.ObjectNode
import com.goatpacker.web.dto.CustomBed
import com.goatpacker.web.dto.ProfileDto
import com.goatpacker.web.exception.BadRequestException

/**
 * Разовый профиль для своего стола: тот же формат, что json-профили каталога (PrinterProfile в Rust),
 * но собран из размеров пользователя. Чего мы о незнакомом принтере не знаем, то выключено:
 * линий прочистки нет (их заменяет [CustomBed.margin]), чистки сопла и AMS-коррекции нет.
 */
object CustomProfile {
    const val ID = "custom"
    const val NAME = "Custom Printer"
    const val BED_FROM_PROJECT = "Размеры стола берутся из проекта — их нельзя изменить"
    const val RECOGNISED_PRINTER = "Принтер этого проекта есть в каталоге — выберите его, свой стол не нужен"

    /** Допуск сверки введённых размеров со столом проекта, мм (поля формы округляют до десятых). */
    private const val PROJECT_BED_TOLERANCE_MM = 0.5

    /** DTO разового профиля по снимку profile.json (серверный джоб хранит только его): тот же вид, что собирает [build]. */
    fun dtoOf(node: com.fasterxml.jackson.databind.JsonNode): ProfileDto {
        val area = node.path("printable_area").map { it.asDouble() }.takeIf { it.size == 4 } ?: listOf(0.0, 0.0, 0.0, 0.0)
        return ProfileDto(
            id = ID, name = node.path("printer_name").asText(NAME), vendor = "", slicerKind = node.path("slicer_kind").asText("orca"),
            bedWidthMm = area[2] - area[0], bedDepthMm = area[3] - area[1], edgeMarginMm = node.path("edge_margin_mm").asDouble(),
            supportsNozzleClean = false, filamentChangeSeconds = null,
            printableArea = area, exclusionRectangles = node.path("exclusion_rectangles").map { r -> r.map { it.asDouble() } },
        )
    }

    data class Built(val dto: ProfileDto, val json: String)

    /**
     * Слайсер по printer_model — зеркало SLICER_KIND_BY_PRINTER_SUBSTRING (src/colored_packer.rs):
     * пакер отвергает профиль, чей slicer_kind спорит с моделью проекта. Незнакомая модель — Orca.
     */
    fun slicerKind(printerModel: String?): String = if (printerModel?.contains("Bambu Lab") == true) "bambu" else "orca"

    /**
     * [printerModel] — printer_model проекта: пакер требует printer_name профиля, равный ему
     * (validate_source_printer), поэтому разовый профиль называется так же; null — «Custom Printer».
     *
     * [projectBed] — стол из 3MF неизвестного принтера. Слайсер режет проект по его собственному
     * printable_area (project_settings.config копируется байт в байт), поэтому стол — всегда стол
     * проекта: его начало координат, зоны исключения и высота. Введённые размеры обязаны с ним
     * совпасть (иначе 400 [BED_FROM_PROJECT]); от пользователя — только отступ от края. Без стола
     * в проекте (STL или нераспознанная геометрия) стол строится от (0, 0) по введённым цифрам.
     */
    fun build(mapper: ObjectMapper, bed: CustomBed, projectBed: ProjectBed?, printerModel: String?, slicerPath: (String) -> String): Built {
        if (projectBed != null) {
            val p = projectBed.printableArea
            val differs = Math.abs(p[2] - p[0] - bed.width) > PROJECT_BED_TOLERANCE_MM || Math.abs(p[3] - p[1] - bed.depth) > PROJECT_BED_TOLERANCE_MM ||
                (projectBed.printableHeight != null && Math.abs(projectBed.printableHeight - bed.height) > PROJECT_BED_TOLERANCE_MM)
            if (differs) throw BadRequestException(BED_FROM_PROJECT)
        }
        val area = projectBed?.printableArea ?: listOf(0.0, 0.0, bed.width, bed.depth)
        val exclusions = projectBed?.exclusionRectangles ?: emptyList()
        val height = projectBed?.printableHeight ?: bed.height
        val name = printerModel?.trim()?.takeIf { it.isNotEmpty() } ?: NAME
        val kind = slicerKind(name)
        val node: ObjectNode = mapper.createObjectNode().apply {
            put("id", ID); put("printer_name", name); put("download_slug", ID)
            put("slicer_kind", kind); put("slicer_path", slicerPath(kind))
            put("blank_start_gcode_for_cli", false)
            // Башня незнакомого принтера в пробе не проверяется: её размер мы не калибровали.
            put("no_check_for_final_tower_probe", true)
            putArray("printable_area").apply { area.forEach { add(it) } }
            put("printable_height", height)
            put("edge_margin_mm", bed.margin)
            putArray("exclusion_rectangles").apply { exclusions.forEach { r -> addArray().apply { r.forEach { add(it) } } } }
            putArray("purge_lines")
            put("tower_marker_style", "wipe_tower"); put("tower_line_width_mm", 0.5)
        }
        val dto = ProfileDto(
            id = ID, name = name, vendor = "", slicerKind = kind,
            bedWidthMm = area[2] - area[0], bedDepthMm = area[3] - area[1], edgeMarginMm = bed.margin,
            supportsNozzleClean = false, filamentChangeSeconds = null,
            printableArea = area, exclusionRectangles = exclusions,
        )
        return Built(dto, mapper.writeValueAsString(node))
    }
}
