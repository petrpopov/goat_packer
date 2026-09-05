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
    fun `отдаёт три профиля принтеров, отсортированные по имени`() {
        val body = mockMvc.perform(get("/api/profiles"))
            .andExpect(status().isOk)
            .andReturn()
            .response
            .contentAsString

        val profiles = json.readTree(body)
        assertTrue(profiles.isArray, "ожидался массив профилей: $body")
        assertEquals(3, profiles.size(), "ожидалось три профиля: $body")

        val ids = (0 until profiles.size()).map { profiles[it]["id"].asText() }
        // Сортировка по name: "Bambu Lab A2L" < "Bambu Lab P1S" < "Snapmaker U1".
        assertEquals(listOf("bambu_a2l", "bambu_p1s", "snapmaker_u1"), ids)
    }

    @Test
    fun `bambu_a2l содержит размеры стола и поддержку чистки сопла`() {
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
        assertTrue(a2l["supportsNozzleClean"].asBoolean(), "у a2l есть nozzle_clean_gcode")
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
        assertEquals("orca", u1["slicerKind"].asText())
        assertEquals(270.0, u1["bedWidthMm"].asDouble(), 1e-9)
        assertEquals(270.0, u1["bedDepthMm"].asDouble(), 1e-9)
        assertEquals(3.5, u1["edgeMarginMm"].asDouble(), 1e-9)
        assertFalse(u1["supportsNozzleClean"].asBoolean(), "у u1 нет nozzle_clean_gcode")
    }
}
