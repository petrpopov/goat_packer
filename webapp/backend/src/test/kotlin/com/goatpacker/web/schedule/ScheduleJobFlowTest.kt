package com.goatpacker.web.schedule

import com.fasterxml.jackson.databind.ObjectMapper
import com.goatpacker.web.FakeBins
import com.goatpacker.web.miniProject3mf
import com.goatpacker.web.awaitJobCompletion
import java.io.ByteArrayInputStream
import java.nio.file.Files
import java.nio.file.Path
import org.apache.poi.ss.usermodel.WorkbookFactory
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

/** Полный жизненный цикл schedule-джоба на фейковом планировщике, включая packJobId-источник. */
@SpringBootTest
@AutoConfigureMockMvc
class ScheduleJobFlowTest(
    @Autowired private val mockMvc: MockMvc,
) {
    companion object {
        private val tmpRoot: Path = Files.createTempDirectory("goat-schedule-flow")

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

    private fun uploadFile(): MockMultipartFile =
        MockMultipartFile("file", "packed.3mf", MediaType.APPLICATION_OCTET_STREAM_VALUE, miniProject3mf("Bambu Lab A2L"))

    @Test
    fun `расписание из загруженного файла — статус, работы, пластины, xlsx`() {
        val create = mockMvc.perform(
            multipart("/api/schedule/jobs")
                .file(uploadFile())
                .param("profileId", "bambu_a2l"),
        )
            .andExpect(status().isAccepted)
            .andReturn()
        val jobId = json.readTree(create.response.contentAsString)["jobId"].asText()

        val body = mockMvc.awaitJobCompletion(jobId)

        assertEquals("schedule", body["kind"].asText())
        assertEquals("SUCCEEDED", body["status"].asText())
        assertEquals(100, body["progressPercent"].asInt())

        val schedule = body["schedule"]
        assertTrue(schedule != null && schedule.isObject, "schedule-джоба обязана отдать schedule-сводку")
        assertEquals("Bambu Lab A2L", schedule["printer"].asText())
        assertEquals(63, schedule["totalObjects"].asInt())
        assertEquals(1893.5, schedule["totalFilamentGrams"].asDouble(), 1e-9)
        assertEquals("D+1 07:15", schedule["completionClock"].asText())

        val jobs = schedule["jobs"]
        assertTrue(jobs.isArray && jobs.size() == 8, "ожидалось 8 работ: ${jobs}")
        val first = jobs[0]
        assertEquals(1, first["sequence"].asInt())
        assertEquals(7, first["batchSize"].asInt())
        assertEquals("10:00", first["startClock"].asText())
        assertEquals("12:26", first["endClock"].asText())
        assertEquals(36000, first["startSeconds"].asLong())
        assertEquals(44760, first["endSeconds"].asLong())
        assertEquals(41160, first["slicerSeconds"].asLong())
        assertEquals(600, first["preparationSeconds"].asLong())
        assertEquals(7, first["cumulativeObjects"].asInt())
        assertEquals(false, first["overnight"].asBoolean())

        // Пластины: уникальные batch_size в порядке появления, метка — по первой работе батча.
        val plates = schedule["plates"]
        assertEquals(4, plates.size())
        assertEquals("7 pcs @ 10:00", plates[0]["label"].asText())
        assertEquals(7, plates[0]["objectCount"].asInt())
        assertEquals("5 pcs @ 17:48", plates[1]["label"].asText())
        assertEquals("10 pcs @ 21:25", plates[2]["label"].asText())
        assertEquals("12 pcs @ D+1 03:45", plates[3]["label"].asText())
        assertEquals(12, plates[3]["objectCount"].asInt())

        val artifacts = body["artifacts"]
        val names = (0 until artifacts.size()).map { artifacts[it]["name"].asText() }
        assertTrue(
            names.containsAll(listOf("schedule.csv", "schedule.xlsx", "plates.3mf", "report.json", "log.txt")),
            "артефакты: $names",
        )

        // schedule.xlsx: больше 4 КБ и открывается WorkbookFactory, лист «Расписание».
        val xlsxEntry = (0 until artifacts.size()).map { artifacts[it] }
            .first { it["name"].asText() == "schedule.xlsx" }
        assertTrue(xlsxEntry["sizeBytes"].asLong() > 4096, "xlsx должен быть больше 4 КБ")
        val xlsxBytes = mockMvc.perform(get("/api/jobs/$jobId/artifacts/schedule.xlsx"))
            .andExpect(status().isOk)
            .andReturn()
            .response
            .contentAsByteArray
        WorkbookFactory.create(ByteArrayInputStream(xlsxBytes)).use { workbook ->
            assertEquals("Расписание", workbook.getSheetName(0))
            val header = workbook.getSheetAt(0).getRow(0)
            assertEquals("printer", header.getCell(0).stringCellValue)
            assertEquals("overnight", header.getCell(11).stringCellValue)
            assertEquals("Bambu Lab A2L", workbook.getSheetAt(0).getRow(1).getCell(0).stringCellValue)
        }
    }

    @Test
    fun `расписание из packJobId успешной упаковки`() {
        // 1. Сначала прогоняем pack-джобу.
        val packCreate = mockMvc.perform(
            multipart("/api/pack/jobs")
                .file(MockMultipartFile("file", "goat.3mf", MediaType.APPLICATION_OCTET_STREAM_VALUE, miniProject3mf("Bambu Lab A2L")))
                .param("profileId", "bambu_a2l"),
        )
            .andExpect(status().isAccepted)
            .andReturn()
        val packJobId = json.readTree(packCreate.response.contentAsString)["jobId"].asText()
        mockMvc.awaitJobCompletion(packJobId)

        // 2. Планировщик по её артефакту packed.3mf.
        val create = mockMvc.perform(
            multipart("/api/schedule/jobs")
                .param("packJobId", packJobId)
                .param("profileId", "bambu_a2l")
                .param("benchMode", "full"),
        )
            .andExpect(status().isAccepted)
            .andReturn()
        val jobId = json.readTree(create.response.contentAsString)["jobId"].asText()

        val body = mockMvc.awaitJobCompletion(jobId)

        assertEquals("SUCCEEDED", body["status"].asText())
        val jobs = body["schedule"]["jobs"]
        assertTrue(jobs.isArray && jobs.size() > 0, "работы расписания не должны быть пустыми")
    }
}
