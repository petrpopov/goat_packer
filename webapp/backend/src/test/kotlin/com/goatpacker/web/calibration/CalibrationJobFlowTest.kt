package com.goatpacker.web.calibration

import com.fasterxml.jackson.databind.ObjectMapper
import com.goatpacker.web.FakeBins
import com.goatpacker.web.awaitJobCompletion
import com.goatpacker.web.awaitJobTerminal
import com.goatpacker.web.descriptorProject3mf
import com.goatpacker.web.miniProject3mf
import java.nio.file.Files
import java.nio.file.Path
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test
import org.springframework.beans.factory.annotation.Autowired
import org.springframework.boot.test.autoconfigure.web.servlet.AutoConfigureMockMvc
import org.springframework.boot.test.context.SpringBootTest
import org.springframework.http.MediaType
import org.springframework.mock.web.MockMultipartFile
import org.springframework.test.context.DynamicPropertyRegistry
import org.springframework.test.context.DynamicPropertySource
import org.springframework.test.web.servlet.MockMvc
import org.springframework.test.web.servlet.request.MockMvcRequestBuilders.get
import org.springframework.test.web.servlet.request.MockMvcRequestBuilders.multipart
import org.springframework.test.web.servlet.result.MockMvcResultMatchers.status

/** Calibration-джоба (тест смены AMS) на фейковом ams_calibration: CLI, результат, артефакты, отказы. */
@SpringBootTest
@AutoConfigureMockMvc
class CalibrationJobFlowTest(
    @Autowired private val mockMvc: MockMvc,
) {
    companion object {
        private val tmpRoot: Path = Files.createTempDirectory("goat-calibration-flow")

        @JvmStatic
        @DynamicPropertySource
        fun properties(registry: DynamicPropertyRegistry) {
            registry.add("app.packer-bin") { FakeBins.install(tmpRoot, "fake-packer.sh") }
            registry.add("app.scheduler-bin") { FakeBins.install(tmpRoot, "fake-scheduler.sh") }
            registry.add("app.calibration-bin") { FakeBins.install(tmpRoot, "fake-calibration.sh") }
            registry.add("app.profiles-dir") { "../../profiles" }
            registry.add("app.jobs-root") { tmpRoot.resolve("jobs").toString() }
        }
    }

    private val json = ObjectMapper()

    private fun project(bytes: ByteArray) =
        MockMultipartFile("file", "goat.3mf", MediaType.APPLICATION_OCTET_STREAM_VALUE, bytes)

    private fun start(bytes: ByteArray, profileId: String, variant: String?): String {
        val request = multipart("/api/calibration/jobs").file(project(bytes)).param("profileId", profileId)
        variant?.let { request.param("variant", it) }
        val response = mockMvc.perform(request).andExpect(status().isAccepted).andReturn().response
        return json.readTree(response.contentAsString)["jobId"].asText()
    }

    private fun cliArgs(jobId: String): List<String> =
        Files.readAllLines(tmpRoot.resolve("jobs").resolve(jobId).resolve("calibration-args.txt"))

    @Test
    fun `калибровка — аргументы CLI, значения calibration json, инструкция и артефакты без черновиков`() {
        val jobId = start(miniProject3mf("Bambu Lab P1S"), "bambu_p1s", "perfectionist")
        val body = mockMvc.awaitJobCompletion(jobId)

        assertEquals("calibration", body["kind"].asText())
        assertEquals(100, body["progressPercent"].asInt())
        val args = cliArgs(jobId)
        fun valueOf(flag: String) = args.getOrNull(args.indexOf(flag) + 1)
        assertEquals("input.3mf", valueOf("--template"))
        assertTrue(valueOf("--profile")!!.endsWith("p1s.json"), args.toString())
        assertEquals("perfectionist", valueOf("--variant"))
        assertEquals("out", valueOf("--out-dir"))

        val calibration = body["calibration"]
        assertEquals("bambu_p1s", calibration["profileId"].asText())
        assertEquals("perfectionist", calibration["variant"].asText())
        assertEquals(30, calibration["changesExpected"].asInt())
        assertEquals(30, calibration["changesSliced"].asInt())
        assertEquals(1500.0, calibration["estimateASeconds"].asDouble(), 1e-9)
        assertEquals(2400.0, calibration["estimateBSeconds"].asDouble(), 1e-9)
        assertEquals(20.5, calibration["slicerChangeSeconds"].asDouble(), 1e-9)
        assertTrue(calibration["instructions"].asText().contains("calib_A.3mf"), calibration.toString())
        assertTrue(body["pack"].isNull && body["schedule"].isNull)

        val names = body["artifacts"].map { it["name"].asText() }.toSet()
        assertEquals(setOf("calib_A.3mf", "calib_B.3mf", "instructions.md", "calibration.json", "log.txt"), names)
        val instructions = mockMvc.perform(get("/api/jobs/$jobId/artifacts/instructions.md"))
            .andExpect(status().isOk).andReturn().response
        assertTrue(instructions.contentType!!.startsWith("text/markdown"), instructions.contentType)
        assertEquals("fake calib B\n", mockMvc.perform(get("/api/jobs/$jobId/artifacts/calib_B.3mf"))
            .andExpect(status().isOk).andReturn().response.contentAsString)
        // Черновики слайсинга (out/slicing/) наружу не отдаются ни под каким именем.
        for (name in listOf("slicing", "calib_A_slice.log", "calib_A_cli.3mf")) {
            mockMvc.perform(get("/api/jobs/$jobId/artifacts/$name")).andExpect(status().isNotFound)
        }
    }

    @Test
    fun `вариант по умолчанию — обычный`() {
        val jobId = start(miniProject3mf("Bambu Lab A2L"), "bambu_a2l", null)
        val body = mockMvc.awaitJobCompletion(jobId)
        assertEquals("normal", cliArgs(jobId)[cliArgs(jobId).indexOf("--variant") + 1])
        assertEquals(10, body["calibration"]["changesExpected"].asInt())
    }

    @Test
    fun `проект с одним филаментом — ошибка бинарника без перевода`() {
        val single = descriptorProject3mf("Bambu Lab P1S", mapOf("filament_colour" to listOf("#FFFFFF")))
        val body = mockMvc.awaitJobTerminal(start(single, "bambu_p1s", "normal"))
        assertEquals("FAILED", body["status"].asText())
        assertTrue(
            body["error"].asText().startsWith("для теста нужен проект минимум с двумя филаментами"),
            body["error"].asText(),
        )
        assertEquals(listOf("log.txt"), body["artifacts"].map { it["name"].asText() })
    }

    @Test
    fun `принтер без смены AMS (U1) — 400 калибровка не нужна`() {
        val response = mockMvc.perform(
            multipart("/api/calibration/jobs").file(project(miniProject3mf("Snapmaker U1"))).param("profileId", "snapmaker_u1"),
        ).andExpect(status().isBadRequest).andReturn().response
        assertTrue(response.contentAsString.contains("для этого принтера калибровка не нужна"), response.contentAsString)
    }

    @Test
    fun `неверный вариант, чужой принтер и отсутствие файла — 400`() {
        mockMvc.perform(
            multipart("/api/calibration/jobs").file(project(miniProject3mf("Bambu Lab P1S")))
                .param("profileId", "bambu_p1s").param("variant", "extreme"),
        ).andExpect(status().isBadRequest)
        mockMvc.perform(
            multipart("/api/calibration/jobs").file(project(miniProject3mf("Bambu Lab P1S"))).param("profileId", "bambu_a2l"),
        ).andExpect(status().isBadRequest)
        mockMvc.perform(multipart("/api/calibration/jobs").param("profileId", "bambu_p1s"))
            .andExpect(status().isBadRequest)
    }
}
