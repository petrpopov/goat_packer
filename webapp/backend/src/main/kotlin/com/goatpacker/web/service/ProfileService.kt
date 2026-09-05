package com.goatpacker.web.service

import com.fasterxml.jackson.databind.JsonNode
import com.fasterxml.jackson.databind.ObjectMapper
import com.goatpacker.web.config.AppProperties
import com.goatpacker.web.dto.ProfileDto
import com.goatpacker.web.exception.BadRequestException
import org.slf4j.LoggerFactory
import org.springframework.stereotype.Service
import java.nio.file.Files
import java.nio.file.Path

/**
 * Каталог принтеров: читается из {profiles-dir}/{id}.json при каждом запросе (файлов три, дёшево).
 * Профиль в файле: {"id","printer_name","slicer_kind","printable_area":[x0,y0,x1,y1],
 * "edge_margin_mm","nozzle_clean_gcode"(может не быть)}.
 */
@Service
class ProfileService(
    private val properties: AppProperties,
    private val objectMapper: ObjectMapper,
) {
    companion object {
        private val log = LoggerFactory.getLogger(ProfileService::class.java)
    }

    fun list(): List<ProfileDto> {
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
                .sortedBy { profile -> profile.name }
        }
    }

    fun exists(profileId: String): Boolean =
        profileId.isNotBlank() && Files.isRegularFile(properties.profilesDirPath.resolve("$profileId.json"))

    fun require(profileId: String) {
        if (!exists(profileId)) {
            throw BadRequestException("Неизвестный профиль принтера: «$profileId»")
        }
    }

    private fun parseProfile(file: Path): ProfileDto? = try {
        val node: JsonNode = objectMapper.readTree(file.toFile())
        val area = node.get("printable_area")
        if (area == null || !area.isArray || area.size() < 4) {
            log.warn("Профиль {} без printable_area — пропускаю", file.fileName)
            null
        } else {
            ProfileDto(
                id = node.path("id").asText(file.fileName.toString().removeSuffix(".json")),
                name = node.path("printer_name").asText(""),
                slicerKind = node.path("slicer_kind").asText(""),
                bedWidthMm = area.get(2).asDouble() - area.get(0).asDouble(),
                bedDepthMm = area.get(3).asDouble() - area.get(1).asDouble(),
                edgeMarginMm = node.path("edge_margin_mm").asDouble(0.0),
                supportsNozzleClean = !node.path("nozzle_clean_gcode").isMissingNode &&
                    !node.path("nozzle_clean_gcode").isNull,
            )
        }
    } catch (ex: Exception) {
        log.warn("Профиль {} не читается: {}", file.fileName, ex.message)
        null
    }
}
