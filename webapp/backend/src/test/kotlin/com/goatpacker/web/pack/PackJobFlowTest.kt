package com.goatpacker.web.pack

import com.fasterxml.jackson.databind.ObjectMapper
import com.goatpacker.web.awaitJobCompletion
import com.goatpacker.web.FakeBins
import com.goatpacker.web.miniProject3mf
import java.nio.file.Files
import java.nio.file.Path
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertNotNull
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
import org.springframework.test.web.servlet.request.MockMvcRequestBuilders.delete
import org.springframework.test.web.servlet.request.MockMvcRequestBuilders.get
import org.springframework.test.web.servlet.request.MockMvcRequestBuilders.multipart
import org.springframework.test.web.servlet.result.MockMvcResultMatchers.status

/** Полный жизненный цикл pack-джоба на фейковом пакере: 202 → поллинг → артефакты → DELETE. */
@SpringBootTest
@AutoConfigureMockMvc
class PackJobFlowTest(
    @Autowired private val mockMvc: MockMvc,
) {
    companion object {
        private val tmpRoot: Path = Files.createTempDirectory("goat-pack-flow")

        @JvmStatic
        @DynamicPropertySource
        fun properties(registry: DynamicPropertyRegistry) {
            registry.add("app.packer-bin") { FakeBins.install(tmpRoot, "fake-packer.sh") }
            registry.add("app.scheduler-bin") { FakeBins.install(tmpRoot, "fake-scheduler.sh") }
            registry.add("app.profiles-dir") { "../../profiles" }
            registry.add("app.jobs-root") { tmpRoot.resolve("jobs").toString() }
        }
    }

    private val json = ObjectMapper()

    private fun uploadGoat(): String {
        val create = mockMvc.perform(
            multipart("/api/pack/jobs")
                .file(MockMultipartFile("file", "goat.3mf", MediaType.APPLICATION_OCTET_STREAM_VALUE, miniProject3mf("Bambu Lab A2L")))
                .param("profileId", "bambu_a2l"),
        )
            .andExpect(status().isAccepted)
            .andReturn()

        val jobId = json.readTree(create.response.contentAsString)["jobId"].asText()
        assertTrue(jobId.isNotBlank(), "jobId не должен быть пустым")

        return jobId
    }

    @Test
    fun `полный цикл — загрузка, успех, артефакты, скачивание, удаление`() {
        val jobId = uploadGoat()

        val body = mockMvc.awaitJobCompletion(jobId)

        assertEquals("pack", body["kind"].asText())
        assertEquals("SUCCEEDED", body["status"].asText())
        assertEquals(100, body["progressPercent"].asInt())
        assertEquals(30, body["pack"]["objectCount"].asInt())
        assertEquals("bambu_a2l", body["pack"]["profileId"].asText())
        assertEquals(0.7, body["pack"]["clearanceMm"].asDouble(), 1e-9)
        // Без окна: время записанного стола есть, окна нет, флаги времени не передаются.
        // null-поля Jackson не пишет (non_null): «нет окна» = поле отсутствует.
        assertTrue(body["pack"]["time"].path("windowHours").let { it.isMissingNode || it.isNull }, body["pack"].toString())
        assertEquals(79200.0, body["pack"]["time"]["correctedSeconds"].asDouble(), 1e-9)
        assertEquals(30, body["pack"]["time"]["fullCount"].asInt())
        val args = packerArgs(jobId)
        for (flag in listOf("--time-window-hours", "--ams-correction", "--change-seconds", "--time-margin-pct")) {
            assertTrue(flag !in args, "$flag не задан пользователем: $args")
        }
        assertTrue(body.hasNonNull("phaseMessage"), "phaseMessage должен содержать последнюю строку вывода")
        assertNotNull(body["createdAt"])
        assertNotNull(body["finishedAt"])

        val artifacts = body["artifacts"]
        val names = (0 until artifacts.size()).map { artifacts[it]["name"].asText() }
        assertTrue(names.containsAll(listOf("packed.3mf", "verification.json", "log.txt")), "артефакты: $names")
        val packedEntry = (0 until artifacts.size()).map { artifacts[it] }
            .first { it["name"].asText() == "packed.3mf" }
        assertTrue(packedEntry["sizeBytes"].asLong() > 0)
        assertEquals("/api/jobs/$jobId/artifacts/packed.3mf", packedEntry["downloadUrl"].asText())

        // Скачивание отдаёт ровно те байты, что написал фейковый пакер.
        val downloaded = mockMvc.perform(get("/api/jobs/$jobId/artifacts/packed.3mf"))
            .andExpect(status().isOk)
            .andReturn()
        val bytes = downloaded.response.contentAsByteArray
        assertEquals("fake packed 3mf content for tests\n", String(bytes))
        assertTrue(
            downloaded.response.getHeader("Content-Disposition")!!.contains("attachment"),
            "ожидается Content-Disposition: attachment",
        )

        // Имя не из white-list — 404.
        mockMvc.perform(get("/api/jobs/$jobId/artifacts/secret.txt"))
            .andExpect(status().isNotFound)

        // Удаление: 204, после чего статус 404.
        mockMvc.perform(delete("/api/jobs/$jobId")).andExpect(status().isNoContent)
        mockMvc.perform(get("/api/jobs/$jobId")).andExpect(status().isNotFound)
        mockMvc.perform(get("/api/jobs/$jobId/artifacts/packed.3mf")).andExpect(status().isNotFound)
    }

    @Test
    fun `профиль с id не совпадающим с именем файла (bambu_p1s в p1s json) проходит весь цикл`() {
        // Регрессия: разрешение профиля по пути {id}.json ломалось на bambu_p1s —
        // «Неизвестный профиль принтера». Идентификатор живёт внутри json, а не в имени файла.
        val create = mockMvc.perform(
            multipart("/api/pack/jobs")
                .file(MockMultipartFile("file", "goat.3mf", MediaType.APPLICATION_OCTET_STREAM_VALUE, miniProject3mf("Bambu Lab P1S")))
                .param("profileId", "bambu_p1s"),
        )
            .andExpect(status().isAccepted)
            .andReturn()

        val jobId = json.readTree(create.response.contentAsString)["jobId"].asText()
        val body = mockMvc.awaitJobCompletion(jobId)
        assertEquals("SUCCEEDED", body["status"].asText())
        assertEquals("bambu_p1s", body["pack"]["profileId"].asText())
    }

    private fun packerArgs(jobId: String): List<String> =
        Files.readAllLines(tmpRoot.resolve("jobs").resolve(jobId).resolve("packer-args.txt"))

    @Test
    fun `окно печати и коррекция AMS доходят до CLI, итог стола — в результате`() {
        val create = mockMvc.perform(
            multipart("/api/pack/jobs")
                .file(MockMultipartFile("file", "goat.3mf", MediaType.APPLICATION_OCTET_STREAM_VALUE, miniProject3mf("Bambu Lab A2L")))
                .param("profileId", "bambu_a2l")
                .param("timeWindowHours", "12")
                .param("amsCorrection", "true")
                .param("changeSeconds", "80")
                .param("timeMarginPct", "15"),
        )
            .andExpect(status().isAccepted)
            .andReturn()
        val jobId = json.readTree(create.response.contentAsString)["jobId"].asText()
        val body = mockMvc.awaitJobCompletion(jobId)

        val args = packerArgs(jobId)
        fun valueOf(flag: String) = args.getOrNull(args.indexOf(flag) + 1)?.toDouble()
        assertEquals(12.0, valueOf("--time-window-hours"))
        assertTrue("--ams-correction" in args, args.toString())
        assertEquals(80.0, valueOf("--change-seconds"))
        assertEquals(15.0, valueOf("--time-margin-pct"))

        val pack = body["pack"]
        assertEquals(12, pack["objectCount"].asInt())
        val time = pack["time"]
        assertEquals(12.0, time["windowHours"].asDouble(), 1e-9)
        assertEquals(12, time["chosenCount"].asInt())
        // Полный стол — наибольший count среди замеров (второй, после пробы на 1 объект).
        assertEquals(30, time["fullCount"].asInt())
        assertEquals(41580.0, time["correctedSeconds"].asDouble(), 1e-9)
        assertEquals(95.0, time["realChangeSeconds"].asDouble(), 1e-9)
    }

    @Test
    fun `длительность смены без галочки коррекции не включает её молча`() {
        val create = mockMvc.perform(
            multipart("/api/pack/jobs")
                .file(MockMultipartFile("file", "goat.3mf", MediaType.APPLICATION_OCTET_STREAM_VALUE, miniProject3mf("Bambu Lab A2L")))
                .param("profileId", "bambu_a2l")
                .param("changeSeconds", "80"),
        )
            .andExpect(status().isAccepted)
            .andReturn()
        val jobId = json.readTree(create.response.contentAsString)["jobId"].asText()
        mockMvc.awaitJobCompletion(jobId)
        val args = packerArgs(jobId)
        assertTrue("--change-seconds" !in args && "--ams-correction" !in args, args.toString())
    }

    @Test
    fun `статус неизвестной джобы — 404`() {
        mockMvc.perform(get("/api/jobs/no-such-job")).andExpect(status().isNotFound)
        mockMvc.perform(delete("/api/jobs/no-such-job")).andExpect(status().isNotFound)
    }

    @Test
    fun `файл без zip-структуры отклоняется до постановки джобы`() {
        mockMvc.perform(
            multipart("/api/pack/jobs")
                .file(MockMultipartFile("file", "goat.3mf", MediaType.APPLICATION_OCTET_STREAM_VALUE, "не архив".toByteArray()))
                .param("profileId", "bambu_p1s"),
        )
            .andExpect(status().isBadRequest)
    }

    @Test
    fun `файл неизвестного принтера отклоняется с его именем в сообщении`() {
        val response = mockMvc.perform(
            multipart("/api/pack/jobs")
                .file(MockMultipartFile("file", "goat.3mf", MediaType.APPLICATION_OCTET_STREAM_VALUE, miniProject3mf("Prusa MK4")))
                .param("profileId", "bambu_p1s"),
        )
            .andExpect(status().isBadRequest)
            .andReturn()
        val message = response.response.contentAsString
        assertTrue(message.contains("Prusa MK4"), "в сообщении должно быть имя принтера: $message")
    }

    @Test
    fun `несовпадение профиля и принтера файла отклоняется до постановки джобы`() {
        // Регрессия: раньше это доходило до бинарника и падало сырой строкой
        // "source printer 'Bambu Lab P1S' does not match profile 'Snapmaker U1'".
        mockMvc.perform(
            multipart("/api/pack/jobs")
                .file(MockMultipartFile("file", "goat.3mf", MediaType.APPLICATION_OCTET_STREAM_VALUE, miniProject3mf("Bambu Lab P1S")))
                .param("profileId", "snapmaker_u1"),
        )
            .andExpect(status().isBadRequest)
    }
    @Test
    fun `pack accepts descriptor archive and exposes actual work placements`() {
        val response = mockMvc.perform(multipart("/api/pack/jobs")
            .file(MockMultipartFile("file", "valid.3mf", "application/octet-stream", com.goatpacker.web.descriptorProject3mf("Bambu Lab P1S")))
            .param("profileId", "bambu_p1s").param("nozzleClean", "global,every=1"))
            .andExpect(status().isAccepted).andReturn().response
        val id = json.readTree(response.contentAsString)["jobId"].asText()
        val result = mockMvc.awaitJobCompletion(id)
        assertTrue(result["artifacts"].any { it["name"].asText() == "placements.json" })
        mockMvc.perform(get("/api/jobs/$id/artifacts/placements.json")).andExpect(status().isOk)
    }

    @Test
    fun `duplicate every and invalid height are rejected before packing`() {
        for (spec in listOf("global,every=2,every=3", "global,every=0", "from-z=0", "from-z=1e999", "global,every=999999999999999999999999")) {
            val response = mockMvc.perform(multipart("/api/pack/jobs")
                .file(MockMultipartFile("file", "valid.3mf", "application/octet-stream", miniProject3mf("Bambu Lab P1S")))
                .param("profileId", "bambu_p1s").param("nozzleClean", spec))
                .andExpect(status().isBadRequest).andReturn().response
            assertTrue(response.contentAsString.contains("nozzleClean"), response.contentAsString)
        }
    }

}
