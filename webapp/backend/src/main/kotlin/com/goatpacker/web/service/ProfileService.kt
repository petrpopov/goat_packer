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
import java.util.zip.ZipInputStream

/**
 * Каталог принтеров: при каждом запросе читаются все json-файлы из {profiles-dir} (их три,
 * дёшево). Профиль в файле: {"id","printer_name","slicer_kind","printable_area":[x0,y0,x1,y1],
 * "edge_margin_mm","nozzle_clean_gcode"(может не быть)}.
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
) {
    companion object {
        private val log = LoggerFactory.getLogger(ProfileService::class.java)
    }

    /** Профиль, разрешённый по id: DTO для проверок и путь к файлу для команд бинарников. */
    data class ResolvedProfile(val dto: ProfileDto, val file: Path)

    fun list(): List<ProfileDto> = loadAll().map { it.dto }

    /**
     * Определяет профиль по содержимому загруженного 3MF: из Metadata/project_settings.config
     * берётся printer_model и сопоставляется с printer_name профилей (точное совпадение, затем
     * вхождение в обе стороны). Null = определить не удалось, пусть выбирает руками.
     */
    fun detectFrom3mf(file: MultipartFile): String? {
        val printerModel = readPrinterModel(file) ?: return null
        val profiles = loadAll()
        profiles
            .find { it.dto.name.equals(printerModel, ignoreCase = true) }
            ?.let { return it.dto.id }
        profiles
            .find {
                it.dto.name.isNotBlank() && (
                    it.dto.name.contains(printerModel, ignoreCase = true) ||
                        printerModel.contains(it.dto.name, ignoreCase = true)
                )
            }
            ?.let { return it.dto.id }
        return null
    }

    private fun readPrinterModel(file: MultipartFile): String? = try {
        ZipInputStream(file.inputStream).use { zip ->
            var entry = zip.nextEntry
            while (entry != null) {
                if (entry.name.equals("Metadata/project_settings.config", ignoreCase = true)) {
                    val node = objectMapper.readTree(zip.readBytes())
                    val model = node.path("printer_model").asText("").trim()
                    return model.ifEmpty { null }
                }
                entry = zip.nextEntry
            }
            null
        }
    } catch (ex: Exception) {
        log.warn("Не удалось прочитать printer_model из {}: {}", file.originalFilename, ex.message)
        null
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
        }
    }

    private fun parseProfile(file: Path): ResolvedProfile? = try {
        val node: JsonNode = objectMapper.readTree(file.toFile())
        val area = node.get("printable_area")
        if (area == null || !area.isArray || area.size() < 4) {
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
                supportsNozzleClean = !node.path("nozzle_clean_gcode").isMissingNode &&
                    !node.path("nozzle_clean_gcode").isNull,
            )
            ResolvedProfile(dto, file)
        }
    } catch (ex: Exception) {
        log.warn("Профиль {} не читается: {}", file.fileName, ex.message)
        null
    }
}
