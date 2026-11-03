package com.goatpacker.web.service

import com.fasterxml.jackson.databind.JsonNode
import com.fasterxml.jackson.databind.ObjectMapper
import com.fasterxml.jackson.databind.node.ObjectNode
import com.goatpacker.web.config.AppProperties
import com.goatpacker.web.config.SlicerKind
import com.goatpacker.web.dto.CustomBed
import com.goatpacker.web.dto.ProfileDto
import com.goatpacker.web.exception.BadRequestException
import com.goatpacker.web.exception.ErrorMessages
import com.goatpacker.web.model.BuiltProfile
import com.goatpacker.web.model.Detection
import com.goatpacker.web.model.Inspection
import com.goatpacker.web.model.ProjectBed
import com.goatpacker.web.model.ResolvedProfile
import com.goatpacker.web.model.UploadKind
import org.slf4j.LoggerFactory
import org.springframework.stereotype.Service
import org.springframework.web.multipart.MultipartFile
import java.nio.file.Files
import java.nio.file.Path

/**
 * Каталог принтеров: при каждом запросе читаются все json-файлы из {profiles-dir} (их немного,
 * дёшево). Профиль в файле: {"id","printer_name","slicer_kind","printable_area":[x0,y0,x1,y1],
 * "edge_margin_mm","nozzle_clean_gcode"(может не быть),"filament_change_seconds"(может не быть)}.
 *
 * Идентификатор профиля — поле "id" ВНУТРИ json, а не имя файла: у нас p1s.json несёт id
 * "bambu_p1s", и попытка разрешить профиль путём {id}.json ломалась «неизвестным профилем».
 * [require] возвращает и DTO, и путь к файлу — путь к профилю для команд бинарников берётся
 * только отсюда, чтобы имя файла и id больше нигде не смешивались.
 */
@Service
class ProfileService(
    private val properties: AppProperties,
    private val objectMapper: ObjectMapper,
    private val projects: Project3mf,
) {
    private val log = LoggerFactory.getLogger(javaClass)

    fun list(): List<ProfileDto> = loadAll().map { it.dto }

    /**
     * Определяет профиль по содержимому загруженного 3MF: из Metadata/project_settings.config
     * берётся printer_model и сопоставляется с printer_name профилей (точное совпадение, затем
     * вхождение в обе стороны). profileId = null, когда сопоставить не удалось — тогда
     * printerModel говорит, чей это файл, и из него строится понятное сообщение.
     */
    fun detectUpload(file: MultipartFile): Detection {
        val name = file.originalFilename
        if (!StlInputs.isAcceptedUpload(name)) throw BadRequestException(StlInputs.UPLOAD_EXTENSIONS_MESSAGE)
        // STL — голая сетка без настроек слайсера: принтер всегда выбирает пользователь.
        if (StlInputs.isStl(name)) {
            if (file.isEmpty) throw BadRequestException("Файл .stl пуст или не выбран")
            StlInputs.requireValidStl(file)
            return Detection(null, null, kind = UploadKind.MESH_STL)
        }
        return detect(projects.inspect(file))
    }

    /** Стол из проекта отдаётся только неизвестному принтеру: у распознанного геометрия — из профиля. */
    fun detect(inspection: Inspection): Detection =
        detect(inspection.printerModel).let { if (it.profileId == null) it.copy(projectBed = inspection.projectBed) else it }

    fun detect(printerModel: String?): Detection {
        if (printerModel.isNullOrBlank()) return Detection(null, null)
        val profiles = loadAll()
        val exact = profiles.filter { it.dto.name.equals(printerModel, ignoreCase = true) }
        val matches = exact.ifEmpty { profiles.filter { printerModel.startsWith(it.dto.name + " ", ignoreCase = true) } }
        return Detection(matches.singleOrNull()?.dto?.id, printerModel)
    }

    /** Проект должен быть от принтера выбранного профиля: пакер сверяет printer_model с printer_name. */
    fun requireMatches(inspection: Inspection, profile: ResolvedProfile) {
        projects.requireModel(inspection)
        val detected = detect(inspection.printerModel)
        when {
            detected.printerModel == null -> throw BadRequestException(ErrorMessages.PRINTER_MODEL_MISSING)
            detected.profileId == null -> throw BadRequestException("Для принтера «${detected.printerModel}» не найден однозначный профиль")
            detected.profileId != profile.dto.id ->
                throw BadRequestException("Файл от принтера «${detected.printerModel}», а выбран «${profile.dto.name}»")
        }
    }

    /**
     * Коррекция смены прутка имеет смысл только для принтера с AMS-сменами: у тулчейнджера
     * (в профиле нет filament_change_seconds) CLI её молча проигнорировал бы, а явный
     * change-seconds наоборот включил бы ложную коррекцию — поэтому понятная 400 заранее.
     */
    fun requireAmsCorrectionSupported(profile: ResolvedProfile, amsCorrection: Boolean) {
        if (amsCorrection && profile.dto.filamentChangeSeconds == null) {
            throw BadRequestException("Принтер «${profile.dto.name}» меняет пруток без AMS — коррекция времени смены не нужна")
        }
    }

    /**
     * Калибровка меряет смену прутка AMS; принтеру без filament_change_seconds (тулчейнджер, U1)
     * корректировать нечего — тест был бы пустой тратой двух печатей.
     */
    fun requireCalibrationSupported(profile: ResolvedProfile) {
        if (profile.dto.filamentChangeSeconds == null) throw BadRequestException(CALIBRATION_NOT_NEEDED)
    }

    /**
     * Разовый профиль своего стола (см. [CustomProfile]) для 3MF с [printerModel]: без printer_model
     * пакер проект не примет (сверяет его с printer_name профиля). Слайсер — локальный из настроек
     * (в docker-режиме воркер подменяет путь на слайсер образа).
     */
    fun custom(bed: CustomBed, projectBed: ProjectBed?, printerModel: String?): BuiltProfile {
        bed.validate()
        if (printerModel.isNullOrBlank()) throw BadRequestException(ErrorMessages.PRINTER_MODEL_MISSING)
        return CustomProfile.build(objectMapper, bed, projectBed, printerModel, properties::localSlicer)
    }

    /**
     * Копия каталожного профиля со слайсером из настроек (см. [LocalSlicer]) для локального
     * запуска; null — профиль годится как есть.
     */
    fun localSlicerCopy(profile: ResolvedProfile): String? {
        val node = objectMapper.readTree(profile.file.toFile()) as ObjectNode
        return if (LocalSlicer.localize(properties, node)) objectMapper.writeValueAsString(node) else null
    }

    /** Профиль для локального джоба в [directory]: как есть или копия profile.json (её пишет вызывающий). */
    fun forLocalJob(profile: ResolvedProfile, directory: Path): Pair<ResolvedProfile, String?> =
        localSlicerCopy(profile)?.let { profile.copy(file = directory.resolve("profile.json")) to it } ?: (profile to null)

    fun require(profileId: String): ResolvedProfile =
        loadAll().find { it.dto.id == profileId } ?: throw BadRequestException("Неизвестный профиль принтера: «$profileId»")

    private fun loadAll(): List<ResolvedProfile> {
        val dir = properties.profilesDirPath
        if (!Files.isDirectory(dir)) {
            log.warn("Каталог профилей {} не найден — возвращаю пустой список", dir)
            return emptyList()
        }

        val profiles = Files.list(dir).use { stream ->
            stream
                .filter { it.fileName.toString().endsWith(".json") }
                .toList()
                .mapNotNull(::parseProfile)
                .sortedBy { it.dto.name }
        }
        require(profiles.map { it.dto.id }.distinct().size == profiles.size) { "В каталоге профилей повторяются id принтеров" }
        return profiles
    }

    /**
     * Производитель для группировки: vendor_display (каталог: у профилей OrcaSlicer vendor — имя
     * папки пресетов вроде «BBL», нужное для сборки из STL; printer_catalog relabel пишет рядом
     * «Bambu Lab»), затем vendor (ручные профили), затем printer_name без последнего слова.
     */
    private fun vendorOf(node: JsonNode): String {
        for (key in listOf("vendor_display", "vendor")) {
            node.path(key).asText("").trim().takeIf { it.isNotEmpty() }?.let { return it }
        }
        val name = node.path("printer_name").asText("").trim()
        return name.substringBeforeLast(' ', name).trim()
    }

    private fun parseProfile(file: Path): ResolvedProfile? = try {
        val node = objectMapper.readTree(file.toFile())
        val area = node.get("printable_area")
        if (area == null || !area.isArray || area.size() != AREA_COORDINATES || area.any { !it.isNumber || !it.asDouble().isFinite() }) {
            log.warn("Профиль {} без printable_area — пропускаю", file.fileName)
            null
        } else {
            ResolvedProfile(toDto(node, area, file), file)
        }
    } catch (ex: Exception) {
        log.warn("Профиль {} не читается: {}", file.fileName, ex.message)
        null
    }

    private fun toDto(node: JsonNode, area: JsonNode, file: Path): ProfileDto {
        val dto = ProfileDto(
            id = node.path("id").asText(file.fileName.toString().removeSuffix(".json")),
            name = node.path("printer_name").asText(""),
            vendor = vendorOf(node),
            slicerKind = node.path("slicer_kind").asText(""),
            slicers = SlicerChoice.offered(node),
            bedWidthMm = area.get(2).asDouble() - area.get(0).asDouble(),
            bedDepthMm = area.get(3).asDouble() - area.get(1).asDouble(),
            edgeMarginMm = node.path("edge_margin_mm").asDouble(0.0),
            printableArea = area.map { it.asDouble() },
            exclusionRectangles = node.path("exclusion_rectangles")
                .filter { rect -> rect.isArray && rect.size() == AREA_COORDINATES && rect.all { it.isNumber } }
                .map { rect -> rect.map { it.asDouble() } },
            plateOutline = node.get("plate_outline")
                ?.takeIf { outline -> outline.isArray && outline.size() >= MIN_OUTLINE_POINTS && outline.all(::isPoint) }
                ?.map { point -> point.map { it.asDouble() } },
            supportsNozzleClean = node.path("nozzle_clean_gcode").asText("").isNotBlank(),
            filamentChangeSeconds = node.get("filament_change_seconds")
                ?.takeIf { it.isNumber && it.asDouble().isFinite() && it.asDouble() >= 0 }
                ?.asDouble(),
        )
        require(dto.id.isNotBlank() && dto.name.isNotBlank() && SlicerKind.ALL.containsAll(dto.slicers.map { it.kind })) {
            "Профиль без id, имени или поддерживаемого слайсера"
        }
        require(
            dto.bedWidthMm > 0 && dto.bedDepthMm > 0 && dto.edgeMarginMm.isFinite() &&
                dto.edgeMarginMm >= 0 && dto.edgeMarginMm * 2 < minOf(dto.bedWidthMm, dto.bedDepthMm),
        ) { "Некорректные границы стола" }
        return dto
    }

    private fun isPoint(point: JsonNode) = point.isArray && point.size() == 2 && point.all { it.isNumber }

    companion object {
        /** Ответ на калибровку принтера без AMS-смен (общий для локального и серверного режимов). */
        const val CALIBRATION_NOT_NEEDED = "для этого принтера калибровка не нужна"

        /** [x0, y0, x1, y1]: область стола и зона исключения. */
        private const val AREA_COORDINATES = 4
        private const val MIN_OUTLINE_POINTS = 3
    }
}
