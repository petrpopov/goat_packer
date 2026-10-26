package com.goatpacker.web.service

import com.fasterxml.jackson.databind.JsonNode
import com.fasterxml.jackson.databind.ObjectMapper
import com.goatpacker.web.config.AppProperties
import com.goatpacker.web.dto.ProfileDto
import com.goatpacker.web.exception.BadRequestException
import org.slf4j.LoggerFactory
import org.springframework.stereotype.Service
import org.springframework.web.multipart.MultipartFile
import java.nio.file.Files
import java.nio.file.Path

/**
 * Каталог принтеров: при каждом запросе читаются все json-файлы из {profiles-dir} (их три,
 * дёшево). Профиль в файле: {"id","printer_name","slicer_kind","printable_area":[x0,y0,x1,y1],
 * "edge_margin_mm","nozzle_clean_gcode"(может не быть),"filament_change_seconds"(может не быть)}.
 *
 * Идентификатор профиля — поле "id" ВНУТРИ json, а не имя файла: у нас p1s.json несёт id
 * "bambu_p1s", и попытка разрешить профиль путём {id}.json ломалась «неизвестным профилем».
 * [resolve] возвращает и DTO, и путь к файлу — путь к профилю для команд бинарников берётся
 * только отсюда, чтобы имя файла и id больше нигде не смешивались.
 */
@Service
class ProfileService(
    private val properties: AppProperties,
    private val objectMapper: ObjectMapper,
    private val projects: Project3mf,
) {
    companion object {
        private val log = LoggerFactory.getLogger(ProfileService::class.java)

        /** Ответ на калибровку принтера без AMS-смен (общий для локального и серверного режимов). */
        const val CALIBRATION_NOT_NEEDED = "для этого принтера калибровка не нужна"
    }

    /** Профиль, разрешённый по id: DTO для проверок и путь к файлу для команд бинарников. */
    data class ResolvedProfile(val dto: ProfileDto, val file: Path)

    /**
     * Результат автоопределения профиля по 3MF. [printerModel] сохраняется и при неудаче
     * сопоставления — из него строится внятное «файл от принтера „X“, профиля для него нет»
     * вместо голого null.
     */
    data class Detection(val profileId: String?, val printerModel: String?)

    fun list(): List<ProfileDto> = loadAll().map { it.dto }

    /**
     * Определяет профиль по содержимому загруженного 3MF: из Metadata/project_settings.config
     * берётся printer_model и сопоставляется с printer_name профилей (точное совпадение, затем
     * вхождение в обе стороны). profileId = null, когда сопоставить не удалось — тогда
     * printerModel говорит, чей это файл, и из него строится понятное сообщение.
     */
    fun detectFrom3mf(file: MultipartFile): Detection {
        return detect(projects.inspect(file).printerModel)
    }

    fun detect(printerModel: String?): Detection {
        if (printerModel.isNullOrBlank()) return Detection(null, null)
        val profiles = loadAll()
        val exact = profiles.filter { it.dto.name.equals(printerModel, ignoreCase = true) }
        val matches = if (exact.isNotEmpty()) exact else profiles.filter {
            printerModel.startsWith(it.dto.name + " ", ignoreCase = true)
        }
        return Detection(matches.singleOrNull()?.dto?.id, printerModel)
    }

    fun requireMatches(inspection: Project3mf.Inspection, profile: ResolvedProfile) {
        projects.requireModel(inspection)
        val detected = detect(inspection.printerModel)
        when {
            detected.printerModel == null -> throw BadRequestException("В проекте нет printer_model. Сохраните проект с выбранным принтером в слайсере")
            detected.profileId == null -> throw BadRequestException("Для принтера «${detected.printerModel}» не найден однозначный профиль")
            detected.profileId != profile.dto.id -> throw BadRequestException("Файл от принтера «${detected.printerModel}», а выбран «${profile.dto.name}»")
        }
    }

    /**
     * Коррекция смены прутка имеет смысл только для принтера с AMS-сменами: у тулчейнджера
     * (в профиле нет filament_change_seconds) CLI её молча проигнорировал бы, а явный
     * change-seconds наоборот включил бы ложную коррекцию — поэтому понятная 400 заранее.
     */
    fun requireAmsCorrectionSupported(profile: ResolvedProfile, amsCorrection: Boolean) {
        if (amsCorrection && profile.dto.filamentChangeSeconds == null) {
            throw BadRequestException(
                "Принтер «${profile.dto.name}» меняет пруток без AMS — коррекция времени смены не нужна",
            )
        }
    }

    /**
     * Калибровка меряет смену прутка AMS; принтеру без filament_change_seconds (тулчейнджер, U1)
     * корректировать нечего — тест был бы пустой тратой двух печатей.
     */
    fun requireCalibrationSupported(profile: ResolvedProfile) {
        if (profile.dto.filamentChangeSeconds == null) {
            throw BadRequestException(CALIBRATION_NOT_NEEDED)
        }
    }

    fun exists(profileId: String): Boolean = findById(profileId) != null

    fun require(profileId: String): ResolvedProfile =
        findById(profileId)
            ?: throw BadRequestException("Неизвестный профиль принтера: «$profileId»")

    private fun findById(profileId: String): ResolvedProfile? =
        loadAll().find { it.dto.id == profileId }

    private fun loadAll(): List<ResolvedProfile> {
        val dir = properties.profilesDirPath
        if (!Files.isDirectory(dir)) {
            log.warn("Каталог профилей {} не найден — возвращаю пустой список", dir)
            return emptyList()
        }

        return Files.list(dir).use { stream ->
            stream
                .filter { path -> path.fileName.toString().endsWith(".json") }
                .toList()
                .mapNotNull { file -> parseProfile(file) }
                .sortedBy { profile -> profile.dto.name }
                .also { profiles ->
                    require(profiles.map { it.dto.id }.distinct().size == profiles.size) { "В каталоге профилей повторяются id принтеров" }
                }
        }
    }

    private fun parseProfile(file: Path): ResolvedProfile? = try {
        val node: JsonNode = objectMapper.readTree(file.toFile())
        val area = node.get("printable_area")
        if (area == null || !area.isArray || area.size() != 4 || area.any { !it.isNumber || !it.asDouble().isFinite() }) {
            log.warn("Профиль {} без printable_area — пропускаю", file.fileName)
            null
        } else {
            val dto = ProfileDto(
                id = node.path("id").asText(file.fileName.toString().removeSuffix(".json")),
                name = node.path("printer_name").asText(""),
                slicerKind = node.path("slicer_kind").asText(""),
                bedWidthMm = area.get(2).asDouble() - area.get(0).asDouble(),
                bedDepthMm = area.get(3).asDouble() - area.get(1).asDouble(),
                edgeMarginMm = node.path("edge_margin_mm").asDouble(0.0),
                supportsNozzleClean = node.path("nozzle_clean_gcode").asText("").isNotBlank(),
                filamentChangeSeconds = node.get("filament_change_seconds")
                    ?.takeIf { it.isNumber && it.asDouble().isFinite() && it.asDouble() >= 0 }
                    ?.asDouble(),
            )
            require(dto.id.isNotBlank() && dto.name.isNotBlank() && dto.slicerKind in setOf("bambu", "orca")) { "Профиль без id, имени или поддерживаемого слайсера" }
            require(dto.bedWidthMm > 0 && dto.bedDepthMm > 0 && dto.edgeMarginMm.isFinite()
                && dto.edgeMarginMm >= 0 && dto.edgeMarginMm * 2 < minOf(dto.bedWidthMm, dto.bedDepthMm)) { "Некорректные границы стола" }
            ResolvedProfile(dto, file)
        }
    } catch (ex: Exception) {
        log.warn("Профиль {} не читается: {}", file.fileName, ex.message)
        null
    }
}
