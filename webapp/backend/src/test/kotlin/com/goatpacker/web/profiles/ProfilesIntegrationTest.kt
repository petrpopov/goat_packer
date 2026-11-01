package com.goatpacker.web.profiles

import com.fasterxml.jackson.databind.ObjectMapper
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertFalse
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test
import org.springframework.beans.factory.annotation.Autowired
import org.springframework.boot.test.autoconfigure.web.servlet.AutoConfigureMockMvc
import org.springframework.boot.test.context.SpringBootTest
import org.springframework.test.context.TestPropertySource
import org.springframework.test.web.servlet.MockMvc
import org.springframework.test.web.servlet.request.MockMvcRequestBuilders.get
import org.springframework.test.web.servlet.result.MockMvcResultMatchers.status

/** GET /api/profiles против настоящих профилей репозитория (../../profiles). */
@SpringBootTest
@AutoConfigureMockMvc
@TestPropertySource(properties = ["app.profiles-dir=../../profiles"])
class ProfilesIntegrationTest(
    @Autowired private val mockMvc: MockMvc,
) {
    private val json = ObjectMapper()

    @Test
    fun `отдаёт ручные и каталожные профили, отсортированные по имени`() {
        val body = mockMvc.perform(get("/api/profiles"))
            .andExpect(status().isOk)
            .andReturn()
            .response
            .contentAsString

        val profiles = json.readTree(body)
        assertTrue(profiles.isArray, "ожидался массив профилей: $body")

        val all = (0 until profiles.size()).map { profiles[it] }
        val ids = all.map { it["id"].asText() }
        val names = all.map { it["name"].asText() }
        // Три ручных профиля всегда на месте; остальные — каталог OrcaSlicer (docs/catalog).
        assertTrue(ids.containsAll(listOf("bambu_a2l", "bambu_p1s", "snapmaker_u1")), "нет ручных профилей: $ids")
        assertTrue(all.size > 3, "ожидались и каталожные профили: $ids")
        assertEquals(names.sorted(), names, "профили отсортированы по имени")
        // Автоопределение по printer_name: два профиля одного принтера сделали бы его неоднозначным.
        assertEquals(names.size, names.distinct().size, "повторяются printer_name: $names")
        all.forEach { p ->
            assertTrue(p["slicerKind"].asText() in setOf("orca", "bambu"), "slicer_kind: $p")
            assertTrue(p["bedWidthMm"].asDouble() > 0 && p["bedDepthMm"].asDouble() > 0, "размер стола: $p")
        }
    }

    @Test
    fun `bambu_a2l — размеры стола, чистки сопла нет (код не подтверждён железом)`() {
        val body = mockMvc.perform(get("/api/profiles"))
            .andExpect(status().isOk)
            .andReturn()
            .response
            .contentAsString

        val profiles = json.readTree(body)
        val a2l = (0 until profiles.size()).map { profiles[it] }
            .first { it["id"].asText() == "bambu_a2l" }

        assertEquals("Bambu Lab A2L", a2l["name"].asText())
        assertEquals("bambu", a2l["slicerKind"].asText())
        assertEquals(330.0, a2l["bedWidthMm"].asDouble(), 1e-9)
        assertEquals(320.0, a2l["bedDepthMm"].asDouble(), 1e-9)
        assertEquals(0.0, a2l["edgeMarginMm"].asDouble(), 1e-9)
        assertFalse(a2l["supportsNozzleClean"].asBoolean(), "у a2l нет nozzle_clean_gcode")
        assertEquals(95.0, a2l["filamentChangeSeconds"].asDouble(), 1e-9)
    }

    @Test
    fun `bambu_p1s — id не совпадает с именем файла p1s json, но профиль разрешается`() {
        val body = mockMvc.perform(get("/api/profiles"))
            .andExpect(status().isOk)
            .andReturn()
            .response
            .contentAsString

        val profiles = json.readTree(body)
        val p1s = (0 until profiles.size()).map { profiles[it] }
            .first { it["id"].asText() == "bambu_p1s" }

        assertEquals("Bambu Lab P1S", p1s["name"].asText())
        assertEquals("Bambu Lab", p1s["vendor"].asText())
        // Каталожный Bambu (vendor «BBL» — папка Orca) попадает в ту же группу, что и ручной P1S.
        val a1 = (0 until profiles.size()).map { profiles[it] }.first { it["id"].asText() == "bambu_lab_a1" }
        assertEquals("Bambu Lab", a1["vendor"].asText())
        assertEquals(256.0, p1s["bedWidthMm"].asDouble(), 1e-9)
        assertEquals(256.0, p1s["bedDepthMm"].asDouble(), 1e-9)
        assertTrue(p1s["supportsNozzleClean"].asBoolean(), "у p1s есть nozzle_clean_gcode")
        assertEquals(91.0, p1s["filamentChangeSeconds"].asDouble(), 1e-9)
        assertEquals(listOf(0.0, 0.0, 256.0, 256.0), p1s["printableArea"].map { it.asDouble() })
        assertEquals(listOf(listOf(0.0, 0.0, 18.0, 28.0)), p1s["exclusionRectangles"].map { r -> r.map { it.asDouble() } })
        assertTrue(p1s["plateOutline"].size() >= 8, "P1S plate outline is a polygon")
    }

    @Test
    fun `snapmaker_u1 без чистки сопла, с своей кромкой`() {
        val body = mockMvc.perform(get("/api/profiles"))
            .andExpect(status().isOk)
            .andReturn()
            .response
            .contentAsString

        val profiles = json.readTree(body)
        val u1 = (0 until profiles.size()).map { profiles[it] }
            .first { it["id"].asText() == "snapmaker_u1" }

        assertEquals("Snapmaker U1", u1["name"].asText())
        assertEquals("Snapmaker", u1["vendor"].asText())
        assertEquals("orca", u1["slicerKind"].asText())
        assertEquals(270.0, u1["bedWidthMm"].asDouble(), 1e-9)
        assertEquals(270.0, u1["bedDepthMm"].asDouble(), 1e-9)
        // 4.0, не 3.5: Orca Snapmaker при spiral lifting требует зазор СТРОГО больше 3.5 мм.
        assertEquals(4.0, u1["edgeMarginMm"].asDouble(), 1e-9)
        assertFalse(u1["supportsNozzleClean"].asBoolean(), "у u1 нет nozzle_clean_gcode")
        assertEquals(listOf(0.5, 1.0, 270.5, 271.0), u1["printableArea"].map { it.asDouble() })
        assertTrue(u1["exclusionRectangles"].isEmpty)
        // Тулчейнджер: AMS-смен нет, коррекция не нужна — поле отсутствует (null).
        assertTrue(u1["filamentChangeSeconds"] == null || u1["filamentChangeSeconds"].isNull, u1.toString())
    }
}
