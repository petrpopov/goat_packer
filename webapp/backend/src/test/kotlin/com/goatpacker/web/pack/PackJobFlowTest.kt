package com.goatpacker.web.pack

import com.fasterxml.jackson.databind.ObjectMapper
import com.goatpacker.web.awaitJobCompletion
import com.goatpacker.web.FakeBins
import com.goatpacker.web.descriptorProject3mf
import com.goatpacker.web.miniProject3mf
import com.goatpacker.web.repository.JobStore
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
import org.springframework.test.web.servlet.request.MockMvcRequestBuilders.post
import org.springframework.test.web.servlet.result.MockMvcResultMatchers.status

/** Полный жизненный цикл pack-джоба на фейковом пакере: 202 → поллинг → артефакты → DELETE. */
@SpringBootTest
@AutoConfigureMockMvc
class PackJobFlowTest(
    @Autowired private val mockMvc: MockMvc,
    @Autowired private val store: JobStore,
) {
    companion object {
        private val tmpRoot: Path = Files.createTempDirectory("goat-pack-flow")

        @JvmStatic
        @DynamicPropertySource
        fun properties(registry: DynamicPropertyRegistry) {
            registry.add("app.packer-bin") { FakeBins.install(tmpRoot, "fake-packer.sh") }
            registry.add("app.scheduler-bin") { FakeBins.install(tmpRoot, "fake-scheduler.sh") }
            registry.add("app.jobs-root") { tmpRoot.resolve("jobs").toString() }
        }
    }

    private val json = ObjectMapper()

    private fun uploadGoat(): String {
        val create = mockMvc.perform(
            multipart("/api/pack/jobs")
                .file(MockMultipartFile("file", "goat.3mf", MediaType.APPLICATION_OCTET_STREAM_VALUE, miniProject3mf("Test Wide Bed")))
                .param("profileId", "test_wide_bed"),
        )
            .andExpect(status().isAccepted)
            .andReturn()

        val jobId = json.readTree(create.response.contentAsString)["jobId"].asText()
        assertTrue(jobId.isNotBlank(), "jobId не должен быть пустым")

        return jobId
    }

    private fun packCustom(file: ByteArray, vararg params: Pair<String, String>) =
        mockMvc.perform(
            multipart("/api/pack/jobs")
                .file(MockMultipartFile("file", "goat.3mf", MediaType.APPLICATION_OCTET_STREAM_VALUE, file))
                .apply { params.forEach { (k, v) -> param(k, v) } },
        )

    @Test
    fun `свой стол — разовый профиль в каталоге джоба уходит пакеру`() {
        val create = packCustom(miniProject3mf("Acme Unknown 3000"), "customWidth" to "250", "customDepth" to "210", "customHeight" to "220", "customMargin" to "3")
            .andExpect(status().isAccepted).andReturn()
        val jobId = json.readTree(create.response.contentAsString)["jobId"].asText()
        val body = mockMvc.awaitJobCompletion(jobId)
        assertEquals("custom", body["pack"]["profileId"].asText())
        // Статус для рабочей области по URL: синтетический профиль стола и введённый стол в настройках.
        assertEquals("custom", body["profile"]["id"].asText())
        assertEquals(250.0, body["profile"]["bedWidthMm"].asDouble(), 1e-9)
        assertEquals(210.0, body["profile"]["bedDepthMm"].asDouble(), 1e-9)
        assertEquals(3.0, body["profile"]["edgeMarginMm"].asDouble(), 1e-9)
        assertEquals(3.0, body["params"]["customBed"]["margin"].asDouble(), 1e-9)
        assertEquals(220.0, body["params"]["customBed"]["height"].asDouble(), 1e-9)
        val dir = tmpRoot.resolve("jobs").resolve(jobId)
        val profile = json.readTree(dir.resolve("profile.json").toFile())
        // Пакер сверяет printer_model проекта с printer_name профиля — разовый профиль назван так же.
        assertEquals("Acme Unknown 3000", profile["printer_name"].asText())
        assertEquals(listOf(0.0, 0.0, 250.0, 210.0), profile["printable_area"].map { it.asDouble() })
        assertEquals(3.0, profile["edge_margin_mm"].asDouble(), 1e-9)
        val args = Files.readAllLines(dir.resolve("packer-args.txt"))
        assertEquals(dir.resolve("profile.json").toString(), args[args.indexOf("--profile") + 1])
    }

    @Test
    fun `свой стол — ровно один источник стола и границы размеров`() {
        packCustom(miniProject3mf("Acme Unknown 3000")).andExpect(status().isBadRequest)
        packCustom(miniProject3mf("Acme Unknown 3000"), "profileId" to "test_corner_bed", "customWidth" to "250", "customDepth" to "210", "customHeight" to "220")
            .andExpect(status().isBadRequest)
        packCustom(miniProject3mf("Acme Unknown 3000"), "customWidth" to "250", "customDepth" to "210").andExpect(status().isBadRequest)
        packCustom(miniProject3mf("Acme Unknown 3000"), "customWidth" to "40", "customDepth" to "210", "customHeight" to "220").andExpect(status().isBadRequest)
        packCustom(miniProject3mf("Acme Unknown 3000"), "customWidth" to "250", "customDepth" to "210", "customHeight" to "220", "customMargin" to "31")
            .andExpect(status().isBadRequest)
    }

    @Test
    fun `статус pack-джоба несёт имя файла, профиль и настройки, вход скачивается теми же байтами`() {
        val data = miniProject3mf("Test Wide Bed")
        val create = mockMvc.perform(
            multipart("/api/pack/jobs")
                .file(MockMultipartFile("file", "goat.3mf", MediaType.APPLICATION_OCTET_STREAM_VALUE, data))
                .param("profileId", "test_wide_bed").param("clearance", "1.2").param("finalSlice", "false"),
        ).andExpect(status().isAccepted).andReturn()
        val jobId = json.readTree(create.response.contentAsString)["jobId"].asText()
        val body = mockMvc.awaitJobCompletion(jobId)
        assertEquals("goat.3mf", body["originalName"].asText())
        assertEquals("test_wide_bed", body["profile"]["id"].asText())
        val catalog = json.readTree(mockMvc.perform(get("/api/profiles")).andReturn().response.contentAsString)
        assertEquals(catalog.first { it["id"].asText() == "test_wide_bed" }, body["profile"])
        assertEquals(1.2, body["params"]["clearance"].asDouble(), 1e-9)
        assertEquals(false, body["params"]["finalSlice"].asBoolean())
        assertEquals("search", body["params"]["layout"].asText())
        assertTrue(body["params"]["customBed"].isNull)
        val input = mockMvc.perform(get("/api/jobs/$jobId/input")).andExpect(status().isOk).andReturn().response
        assertTrue(input.contentAsByteArray.contentEquals(data))
        assertTrue(input.getHeader("Content-Disposition")!!.contains("goat.3mf"))
        mockMvc.perform(delete("/api/jobs/$jobId")).andExpect(status().isNoContent)
        mockMvc.perform(get("/api/jobs/$jobId/input")).andExpect(status().isNotFound)
    }

    @Test
    fun `полный цикл — загрузка, успех, артефакты, скачивание, удаление`() {
        val jobId = uploadGoat()

        val body = mockMvc.awaitJobCompletion(jobId)

        assertEquals("pack", body["kind"].asText())
        assertEquals("SUCCEEDED", body["status"].asText())
        assertEquals(100, body["progressPercent"].asInt())
        assertEquals(30, body["pack"]["objectCount"].asInt())
        assertEquals("test_wide_bed", body["pack"]["profileId"].asText())
        assertEquals(0.7, body["pack"]["clearanceMm"].asDouble(), 1e-9)
        // Без окна: время записанного стола есть, окна нет, флаги времени не передаются.
        // null-поля Jackson не пишет (non_null): «нет окна» = поле отсутствует.
        assertTrue(body["pack"]["time"].path("windowHours").let { it.isMissingNode || it.isNull }, body["pack"].toString())
        assertEquals(79200.0, body["pack"]["time"]["correctedSeconds"].asDouble(), 1e-9)
        assertEquals(30, body["pack"]["time"]["fullCount"].asInt())
        // Раскладка пакера (layout-событие) видна и после успеха: по ней рисуется итоговый стол.
        assertEquals(2, body["layout"]["placements"].size(), "раскладка видна и после успеха")
        assertEquals("pack", body["layout"]["source"].asText())
        val args = packerArgs(jobId)
        for (flag in listOf("--time-window-hours", "--ams-correction", "--change-seconds", "--time-margin-pct", "--support-overlap-mm", "--tower-width")) {
            assertTrue(flag !in args, "$flag не задан пользователем: $args")
        }
        // Отчёт без полей поддержек (старый пакер) — их нет и в результате, ошибки нет.
        assertTrue(!body["pack"].hasNonNull("supportsDetected"), body["pack"].toString())
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
    fun `профиль с id не совпадающим с именем файла (test_corner_bed в corner_bed json) проходит весь цикл`() {
        // Регрессия: разрешение профиля по пути {id}.json ломалось на профиле, чей файл назван не по id, —
        // «Неизвестный профиль принтера». Идентификатор живёт внутри json, а не в имени файла.
        val create = mockMvc.perform(
            multipart("/api/pack/jobs")
                .file(MockMultipartFile("file", "goat.3mf", MediaType.APPLICATION_OCTET_STREAM_VALUE, miniProject3mf("Test Corner Bed")))
                .param("profileId", "test_corner_bed"),
        )
            .andExpect(status().isAccepted)
            .andReturn()

        val jobId = json.readTree(create.response.contentAsString)["jobId"].asText()
        val body = mockMvc.awaitJobCompletion(jobId)
        assertEquals("SUCCEEDED", body["status"].asText())
        assertEquals("test_corner_bed", body["pack"]["profileId"].asText())
    }

    private fun packerArgs(jobId: String): List<String> =
        Files.readAllLines(tmpRoot.resolve("jobs").resolve(jobId).resolve("packer-args.txt"))

    @Test
    fun `окно печати и коррекция AMS доходят до CLI, итог стола — в результате`() {
        val create = mockMvc.perform(
            multipart("/api/pack/jobs")
                .file(MockMultipartFile("file", "goat.3mf", MediaType.APPLICATION_OCTET_STREAM_VALUE, miniProject3mf("Test Wide Bed")))
                .param("profileId", "test_wide_bed")
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
    fun `перекрытие поддержек доходит до CLI, обнаруженные поддержки — в результате`() {
        for (overlap in listOf("0", "2.5")) {
            val create = mockMvc.perform(
                multipart("/api/pack/jobs")
                    .file(MockMultipartFile("file", "goat.3mf", MediaType.APPLICATION_OCTET_STREAM_VALUE, miniProject3mf("Test Wide Bed")))
                    .param("profileId", "test_wide_bed")
                    .param("supportOverlapMm", overlap)
                    .param("towerWidth", "12.5"),
            )
                .andExpect(status().isAccepted)
                .andReturn()
            val jobId = json.readTree(create.response.contentAsString)["jobId"].asText()
            val body = mockMvc.awaitJobCompletion(jobId)
            val args = packerArgs(jobId)
            assertEquals(overlap.toDouble(), args[args.indexOf("--support-overlap-mm") + 1].toDouble())
            // Ширина башни доходит до пакера и возвращается в настройках (форма только для чтения).
            assertEquals(12.5, args[args.indexOf("--tower-width") + 1].toDouble())
            assertEquals(12.5, body["params"]["towerWidth"].asDouble(), 1e-9)
            assertTrue(body["pack"]["supportsDetected"].asBoolean(), body["pack"].toString())
            assertEquals(overlap.toDouble(), body["pack"]["supportOverlapMm"].asDouble(), 1e-9)
        }
    }

    @Test
    fun `упаковка без полной нарезки и доводка «Нарезать стол»`() {
        val create = mockMvc.perform(
            multipart("/api/pack/jobs")
                .file(MockMultipartFile("file", "goat.3mf", MediaType.APPLICATION_OCTET_STREAM_VALUE, miniProject3mf("Test Wide Bed")))
                .param("profileId", "test_wide_bed")
                .param("finalSlice", "false"),
        )
            .andExpect(status().isAccepted)
            .andReturn()
        val packId = json.readTree(create.response.contentAsString)["jobId"].asText()
        val packed = mockMvc.awaitJobCompletion(packId)
        assertTrue("--verify" !in packerArgs(packId))
        assertEquals(false, packed["pack"]["finalSliced"].asBoolean())
        // Башня из отчёта (tower.anchor/size): по ней фронт просит вернуть башню в угол после открытия в GUI.
        val tower = packed["pack"]["tower"]
        assertEquals(10.0, tower["anchorXMm"].asDouble(), 1e-9)
        assertEquals(10.0, tower["anchorYMm"].asDouble(), 1e-9)
        assertEquals(20.0, tower["widthMm"].asDouble(), 1e-9)
        assertTrue(packed["artifacts"].any { it["name"].asText() == "pack_state.json" }, packed["artifacts"].toString())

        // Проверка нарезкой остаётся внутри упаковки: id тот же, внутренняя задача наружу не светится.
        val finish = mockMvc.perform(post("/api/pack/jobs/$packId/finish"))
            .andExpect(status().isAccepted)
            .andReturn()
        assertEquals(packId, json.readTree(finish.response.contentAsString)["jobId"].asText())
        val checkId = store.get(packId)!!.checkJobId!!
        assertTrue(checkId != packId)
        val deadline = System.currentTimeMillis() + 30_000
        var withCheck = json.readTree(mockMvc.perform(get("/api/jobs/$packId")).andReturn().response.contentAsString)
        while (withCheck["check"].isNull || withCheck["check"]["status"].asText() !in setOf("SUCCEEDED", "FAILED")) {
            assertTrue(System.currentTimeMillis() < deadline, withCheck.toString())
            Thread.sleep(50)
            withCheck = json.readTree(mockMvc.perform(get("/api/jobs/$packId")).andReturn().response.contentAsString)
        }
        val check = withCheck["check"]
        assertEquals("SUCCEEDED", check["status"].asText(), withCheck.toString())
        assertEquals(true, check["pack"]["finalSliced"].asBoolean())
        assertTrue(!withCheck.toString().contains(checkId), "id внутренней задачи не должен попадать в ответ")
        assertTrue(check["artifacts"].all { it["downloadUrl"].asText().startsWith("/api/jobs/$packId/check/artifacts/") }, check.toString())
        val args = packerArgs(checkId)
        assertEquals("finish.state.json", args[args.indexOf("--finish-state") + 1])
        assertTrue("--verify" in args, args.toString())
        // Артефакт проверки скачивается по id упаковки; внутренняя задача по своему id не видна.
        mockMvc.perform(get(check["artifacts"][0]["downloadUrl"].asText())).andExpect(status().isOk)
        mockMvc.perform(get("/api/jobs/$checkId")).andExpect(status().isNotFound)
        // Стол уже проверен: повторно нельзя; отмены у завершённой проверки нет — она остаётся.
        mockMvc.perform(post("/api/pack/jobs/$packId/finish")).andExpect(status().isBadRequest)
        mockMvc.perform(post("/api/pack/jobs/$packId/check/cancel")).andExpect(status().isNoContent)
        assertEquals("SUCCEEDED", json.readTree(mockMvc.perform(get("/api/jobs/$packId")).andReturn().response.contentAsString)["check"]["status"].asText())
        // Удаление упаковки уносит и проверку.
        mockMvc.perform(delete("/api/jobs/$packId")).andExpect(status().isNoContent)
        assertEquals(null, store.get(checkId))
    }

    @Test
    fun `длительность смены без галочки коррекции не включает её молча`() {
        val create = mockMvc.perform(
            multipart("/api/pack/jobs")
                .file(MockMultipartFile("file", "goat.3mf", MediaType.APPLICATION_OCTET_STREAM_VALUE, miniProject3mf("Test Wide Bed")))
                .param("profileId", "test_wide_bed")
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
                .param("profileId", "test_corner_bed"),
        )
            .andExpect(status().isBadRequest)
    }

    @Test
    fun `файл неизвестного принтера отклоняется с его именем в сообщении`() {
        val response = mockMvc.perform(
            multipart("/api/pack/jobs")
                .file(MockMultipartFile("file", "goat.3mf", MediaType.APPLICATION_OCTET_STREAM_VALUE, miniProject3mf("Acme Unknown 3000")))
                .param("profileId", "test_corner_bed"),
        )
            .andExpect(status().isBadRequest)
            .andReturn()
        val message = response.response.contentAsString
        assertTrue(message.contains("Acme Unknown 3000"), "в сообщении должно быть имя принтера: $message")
    }

    @Test
    fun `несовпадение профиля и принтера файла отклоняется до постановки джобы`() {
        // Регрессия: раньше это доходило до бинарника и падало сырой строкой
        // "source printer 'Test Corner Bed' does not match profile 'Test Margin Bed'".
        mockMvc.perform(
            multipart("/api/pack/jobs")
                .file(MockMultipartFile("file", "goat.3mf", MediaType.APPLICATION_OCTET_STREAM_VALUE, miniProject3mf("Test Corner Bed")))
                .param("profileId", "test_margin_bed"),
        )
            .andExpect(status().isBadRequest)
    }
    @Test
    fun `pack accepts descriptor archive and exposes actual work placements`() {
        val response = mockMvc.perform(multipart("/api/pack/jobs")
            .file(MockMultipartFile("file", "valid.3mf", "application/octet-stream", descriptorProject3mf("Test Corner Bed")))
            .param("profileId", "test_corner_bed").param("nozzleClean", "global,every=1"))
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
                .file(MockMultipartFile("file", "valid.3mf", "application/octet-stream", miniProject3mf("Test Corner Bed")))
                .param("profileId", "test_corner_bed").param("nozzleClean", spec))
                .andExpect(status().isBadRequest).andReturn().response
            assertTrue(response.contentAsString.contains("nozzleClean"), response.contentAsString)
        }
    }

}
