package com.goatpacker.web.pack

import com.fasterxml.jackson.databind.JsonNode
import com.fasterxml.jackson.databind.ObjectMapper
import com.goatpacker.web.FakeBins
import com.goatpacker.web.awaitJobCompletion
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertFalse
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
import java.nio.file.Files
import java.nio.file.Path

/** STL в локальном режиме: source_project (фейк) собирает input.3mf, затем тот же пакер. */
@SpringBootTest
@AutoConfigureMockMvc
class StlPackFlowTest(@Autowired private val mockMvc: MockMvc) {
    companion object {
        private val tmpRoot: Path = Files.createTempDirectory("goat-stl-flow")
        private val presets: Path = Files.createDirectories(tmpRoot.resolve("orca-profiles"))

        @JvmStatic
        @DynamicPropertySource
        fun properties(registry: DynamicPropertyRegistry) {
            registry.add("app.packer-bin") { FakeBins.install(tmpRoot, "fake-packer.sh") }
            registry.add("app.source-project-bin") { FakeBins.install(tmpRoot, "fake-source-project.sh") }
            registry.add("app.slicer-presets") { presets.toString() }
            registry.add("app.orca-slicer") { "/fake/orca" }
            registry.add("app.bambu-slicer") { "/fake/bambu" }
            registry.add("app.profiles-dir") { "../../profiles" }
            registry.add("app.jobs-root") { tmpRoot.resolve("jobs").toString() }
        }
    }

    private val json = ObjectMapper()
    private val cube = "solid cube\nendsolid cube\n".toByteArray()

    private fun pack(stl: ByteArray, vararg params: Pair<String, String>): String {
        val created = mockMvc.perform(
            multipart("/api/pack/jobs")
                .file(MockMultipartFile("file", "cube.stl", MediaType.APPLICATION_OCTET_STREAM_VALUE, stl))
                .apply { params.forEach { (k, v) -> param(k, v) } },
        ).andExpect(status().isAccepted).andReturn()
        return json.readTree(created.response.contentAsString)["jobId"].asText()
    }

    private fun dir(jobId: String) = tmpRoot.resolve("jobs").resolve(jobId)
    private fun lines(jobId: String, file: String) = Files.readAllLines(dir(jobId).resolve(file))
    private fun List<String>.after(flag: String) = this[indexOf(flag) + 1]

    @Test
    fun `STL на принтере каталога — source_project с пресетом машины, затем пакер по input_3mf`() {
        val jobId = pack(cube, "profileId" to "prusa_mk4s")
        val body = mockMvc.awaitJobCompletion(jobId)
        assertEquals(30, body["pack"]["objectCount"].asInt())
        val source = lines(jobId, "source-args.txt")
        assertEquals(
            listOf("--stl", "input.stl", "--presets", presets.toString(), "--orca", "/fake/orca", "--vendor", "Prusa",
                "--machine", "Prusa MK4S 0.4 nozzle", "--output", "input.3mf", "--work-dir", "stl-work"),
            source,
        )
        assertTrue(Files.isRegularFile(dir(jobId).resolve("input.stl")))
        assertTrue(Files.isRegularFile(dir(jobId).resolve("input.3mf")))
        val packer = lines(jobId, "packer-args.txt")
        assertEquals("input.3mf", packer.after("--source"))
        // Каталожный профиль указывает слайсер runner-образа — локально джоб получает копию со своим.
        val profile = json.readTree(Path.of(packer.after("--profile")).toFile())
        assertEquals("/fake/orca", profile["slicer_path"].asText())
        assertEquals("prusa_mk4s", profile["id"].asText())
    }

    @Test
    fun `STL на своём столе — custom-bed и разовый профиль Custom Printer`() {
        val jobId = pack(cube, "customWidth" to "220", "customDepth" to "220", "customHeight" to "250", "customMargin" to "5")
        val body = mockMvc.awaitJobCompletion(jobId)
        assertEquals("custom", body["pack"]["profileId"].asText())
        val source = lines(jobId, "source-args.txt")
        assertEquals("220,220,250", source.after("--custom-bed"))
        assertFalse("--vendor" in source, source.toString())
        val profile = json.readTree(dir(jobId).resolve("profile.json").toFile())
        // printer_model проекта из source_project — «Custom Printer»; пакер требует такое же printer_name.
        assertEquals("Custom Printer", profile["printer_name"].asText())
        assertEquals(250.0, profile["printable_height"].asDouble(), 1e-9)
    }

    @Test
    fun `STL на Bambu — пресет BBL, без дерева Bambu Studio экспортирует Orca, пакует Bambu Studio`() {
        val jobId = pack(cube, "profileId" to "bambu_p1s")
        mockMvc.awaitJobCompletion(jobId)
        val source = lines(jobId, "source-args.txt")
        assertEquals("BBL", source.after("--vendor"))
        assertEquals("Bambu Lab P1S 0.4 nozzle", source.after("--machine"))
        assertEquals("/fake/orca", source.after("--orca"))
    }

    @Test
    fun `ошибка конвертации — понятная причина, пакер не запускается`() {
        val jobId = pack("solid FAIL\nendsolid\n".toByteArray(), "profileId" to "prusa_mk4s")
        val body = awaitFailure(jobId)
        val error = body["error"].asText()
        assertTrue(error.startsWith("Не удалось подготовить проект из STL: the model's footprint"), error)
        assertFalse(Files.exists(dir(jobId).resolve("packer-args.txt")), "пакер не должен запускаться")
    }

    private fun awaitFailure(jobId: String): JsonNode {
        val deadline = System.currentTimeMillis() + 30_000
        while (System.currentTimeMillis() < deadline) {
            val body = json.readTree(mockMvc.perform(get("/api/jobs/$jobId")).andReturn().response.contentAsString)
            when (body["status"].asText()) {
                "FAILED" -> return body
                "SUCCEEDED" -> throw AssertionError("ожидался провал: $body")
            }
            Thread.sleep(100)
        }
        throw AssertionError("джоба $jobId не завершилась")
    }
}
