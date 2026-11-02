package com.goatpacker.web.profiles

import com.fasterxml.jackson.databind.JsonNode
import com.fasterxml.jackson.databind.ObjectMapper
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertFalse
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test
import org.springframework.beans.factory.annotation.Autowired
import org.springframework.boot.test.autoconfigure.web.servlet.AutoConfigureMockMvc
import org.springframework.boot.test.context.SpringBootTest
import org.springframework.test.web.servlet.MockMvc
import org.springframework.test.web.servlet.request.MockMvcRequestBuilders.get
import org.springframework.test.web.servlet.result.MockMvcResultMatchers.status

/**
 * GET /api/profiles против тестового каталога (src/test/resources/profiles, generic-принтеры):
 * ручные профили (стол с углом-исключением, широкий стол, стол с кромкой) и один «каталожный»
 * в формате printer_catalog. Настоящий каталог проверяет [CatalogProfilesLoadTest].
 */
@SpringBootTest
@AutoConfigureMockMvc
class ProfilesIntegrationTest(
    @Autowired private val mockMvc: MockMvc,
) {
    private val json = ObjectMapper()

    private fun profiles(): List<JsonNode> {
        val body = mockMvc.perform(get("/api/profiles"))
            .andExpect(status().isOk)
            .andReturn()
            .response
            .contentAsString
        val profiles = json.readTree(body)
        assertTrue(profiles.isArray, "ожидался массив профилей: $body")
        return (0 until profiles.size()).map { profiles[it] }
    }

    private fun profile(id: String) = profiles().first { it["id"].asText() == id }

    @Test
    fun `отдаёт ручные и каталожные профили, отсортированные по имени`() {
        val all = profiles()
        val ids = all.map { it["id"].asText() }
        val names = all.map { it["name"].asText() }
        assertEquals(setOf("test_corner_bed", "test_wide_bed", "test_margin_bed", "test_catalog_bed"), ids.toSet(), "профили: $ids")
        assertEquals(names.sorted(), names, "профили отсортированы по имени")
        // Автоопределение по printer_name: два профиля одного принтера сделали бы его неоднозначным.
        assertEquals(names.size, names.distinct().size, "повторяются printer_name: $names")
        all.forEach { p ->
            assertTrue(p["slicerKind"].asText() in setOf("orca", "bambu"), "slicer_kind: $p")
            assertTrue(p["bedWidthMm"].asDouble() > 0 && p["bedDepthMm"].asDouble() > 0, "размер стола: $p")
        }
    }

    @Test
    fun `широкий стол — размеры, чистки сопла нет, смена прутка из профиля`() {
        val wide = profile("test_wide_bed")
        assertEquals("Test Wide Bed", wide["name"].asText())
        assertEquals("bambu", wide["slicerKind"].asText())
        assertEquals(330.0, wide["bedWidthMm"].asDouble(), 1e-9)
        assertEquals(320.0, wide["bedDepthMm"].asDouble(), 1e-9)
        assertEquals(0.0, wide["edgeMarginMm"].asDouble(), 1e-9)
        assertFalse(wide["supportsNozzleClean"].asBoolean(), "у широкого стола нет nozzle_clean_gcode")
        assertEquals(95.0, wide["filamentChangeSeconds"].asDouble(), 1e-9)
    }

    @Test
    fun `стол с углом — id не совпадает с именем файла corner_bed json, но профиль разрешается`() {
        val corner = profile("test_corner_bed")
        assertEquals("Test Corner Bed", corner["name"].asText())
        assertEquals("Test Vendor", corner["vendor"].asText())
        // Каталожный профиль (vendor — папка пресетов Orca) попадает в ту же группу, что и ручной.
        assertEquals("Test Vendor", profile("test_catalog_bed")["vendor"].asText())
        assertEquals(256.0, corner["bedWidthMm"].asDouble(), 1e-9)
        assertEquals(256.0, corner["bedDepthMm"].asDouble(), 1e-9)
        assertTrue(corner["supportsNozzleClean"].asBoolean(), "у стола с углом есть nozzle_clean_gcode")
        assertEquals(91.0, corner["filamentChangeSeconds"].asDouble(), 1e-9)
        assertEquals(listOf(0.0, 0.0, 256.0, 256.0), corner["printableArea"].map { it.asDouble() })
        assertEquals(listOf(listOf(0.0, 0.0, 18.0, 28.0)), corner["exclusionRectangles"].map { r -> r.map { it.asDouble() } })
        assertTrue(corner["plateOutline"].size() >= 8, "plate outline is a polygon")
    }

    @Test
    fun `стол с кромкой без чистки сопла, со своей кромкой`() {
        val margin = profile("test_margin_bed")
        assertEquals("Test Margin Bed", margin["name"].asText())
        assertEquals("Test Tools", margin["vendor"].asText())
        assertEquals("orca", margin["slicerKind"].asText())
        assertEquals(270.0, margin["bedWidthMm"].asDouble(), 1e-9)
        assertEquals(270.0, margin["bedDepthMm"].asDouble(), 1e-9)
        assertEquals(4.0, margin["edgeMarginMm"].asDouble(), 1e-9)
        assertFalse(margin["supportsNozzleClean"].asBoolean(), "у стола с кромкой нет nozzle_clean_gcode")
        assertEquals(listOf(0.5, 1.0, 270.5, 271.0), margin["printableArea"].map { it.asDouble() })
        assertTrue(margin["exclusionRectangles"].isEmpty)
        // Тулчейнджер: AMS-смен нет, коррекция не нужна — поле отсутствует (null).
        assertTrue(margin["filamentChangeSeconds"] == null || margin["filamentChangeSeconds"].isNull, margin.toString())
    }
}
