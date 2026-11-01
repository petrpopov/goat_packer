package com.goatpacker.web.service

import com.fasterxml.jackson.databind.JsonNode
import com.fasterxml.jackson.databind.ObjectMapper
import com.fasterxml.jackson.databind.node.ObjectNode
import com.goatpacker.web.config.SlicerKind
import com.goatpacker.web.dto.CustomBed
import com.goatpacker.web.dto.ProfileDto
import com.goatpacker.web.exception.BadRequestException
import com.goatpacker.web.model.BuiltProfile
import com.goatpacker.web.model.ProjectBed
import kotlin.math.abs

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

    /** Подпись принтеров Bambu Lab в `printer_model`: слайсер по ней отличается от Orca. */
    private const val BAMBU_MODEL_MARKER = "Bambu Lab"

    /** Ширина линии маркера башни очистки на сцене, мм. */
    private const val TOWER_LINE_WIDTH_MM = 0.5

    /** DTO разового профиля по снимку profile.json (серверный джоб хранит только его): тот же вид, что собирает [build]. */
    fun dtoOf(node: JsonNode): ProfileDto {
        val area = node.path("printable_area").map { it.asDouble() }.takeIf { it.size == 4 } ?: listOf(0.0, 0.0, 0.0, 0.0)
        return dto(
            name = node.path("printer_name").asText(NAME),
            slicerKind = node.path("slicer_kind").asText(SlicerKind.ORCA),
            area = area,
            margin = node.path("edge_margin_mm").asDouble(),
            exclusions = node.path("exclusion_rectangles").map { rect -> rect.map { it.asDouble() } },
        )
    }

    /**
     * Слайсер по printer_model — зеркало SLICER_KIND_BY_PRINTER_SUBSTRING (src/colored_packer.rs):
     * пакер отвергает профиль, чей slicer_kind спорит с моделью проекта. Незнакомая модель — Orca.
     */
    fun slicerKind(printerModel: String?): String =
        if (printerModel?.contains(BAMBU_MODEL_MARKER) == true) SlicerKind.BAMBU else SlicerKind.ORCA

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
    fun build(
        mapper: ObjectMapper,
        bed: CustomBed,
        projectBed: ProjectBed?,
        printerModel: String?,
        slicerPath: (String) -> String,
    ): BuiltProfile {
        if (projectBed != null && differsFromProject(bed, projectBed)) throw BadRequestException(BED_FROM_PROJECT)
        val area = projectBed?.printableArea ?: listOf(0.0, 0.0, bed.width, bed.depth)
        val exclusions = projectBed?.exclusionRectangles ?: emptyList()
        val height = projectBed?.printableHeight ?: bed.height
        val name = printerModel?.trim()?.takeIf { it.isNotEmpty() } ?: NAME
        val kind = slicerKind(name)

        val node: ObjectNode = mapper.createObjectNode().apply {
            put("id", ID)
            put("printer_name", name)
            put("download_slug", ID)
            put("slicer_kind", kind)
            put("slicer_path", slicerPath(kind))
            put("blank_start_gcode_for_cli", false)
            // Башня незнакомого принтера в пробе не проверяется: её размер мы не калибровали.
            put("no_check_for_final_tower_probe", true)
            putArray("printable_area").apply { area.forEach { add(it) } }
            put("printable_height", height)
            put("edge_margin_mm", bed.margin)
            putArray("exclusion_rectangles").apply { exclusions.forEach { rect -> addArray().apply { rect.forEach { add(it) } } } }
            putArray("purge_lines")
            put("tower_marker_style", "wipe_tower")
            put("tower_line_width_mm", TOWER_LINE_WIDTH_MM)
        }
        return BuiltProfile(dto(name, kind, area, bed.margin, exclusions), mapper.writeValueAsString(node))
    }

    private fun differsFromProject(bed: CustomBed, projectBed: ProjectBed): Boolean {
        val area = projectBed.printableArea
        return abs(area[2] - area[0] - bed.width) > PROJECT_BED_TOLERANCE_MM ||
            abs(area[3] - area[1] - bed.depth) > PROJECT_BED_TOLERANCE_MM ||
            (projectBed.printableHeight != null && abs(projectBed.printableHeight - bed.height) > PROJECT_BED_TOLERANCE_MM)
    }

    private fun dto(name: String, slicerKind: String, area: List<Double>, margin: Double, exclusions: List<List<Double>>) = ProfileDto(
        id = ID,
        name = name,
        vendor = "",
        slicerKind = slicerKind,
        bedWidthMm = area[2] - area[0],
        bedDepthMm = area[3] - area[1],
        edgeMarginMm = margin,
        supportsNozzleClean = false,
        filamentChangeSeconds = null,
        printableArea = area,
        exclusionRectangles = exclusions,
    )
}
