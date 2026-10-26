package com.goatpacker.web.validation

import com.goatpacker.web.miniProject3mf
import com.goatpacker.web.FakeBins
import java.nio.file.Files
import java.nio.file.Path
import org.junit.jupiter.api.Test
import org.springframework.beans.factory.annotation.Autowired
import org.springframework.boot.test.autoconfigure.web.servlet.AutoConfigureMockMvc
import org.springframework.boot.test.context.SpringBootTest
import org.springframework.http.MediaType
import org.springframework.mock.web.MockMultipartFile
import org.springframework.test.context.DynamicPropertyRegistry
import org.springframework.test.context.DynamicPropertySource
import org.springframework.test.web.servlet.MockMvc
import org.springframework.test.web.servlet.request.MockMvcRequestBuilders.multipart
import org.springframework.test.web.servlet.result.MockMvcResultMatchers.status

/** Валидация форм: все перечисленные в контракте случаи обязаны давать 400. */
@SpringBootTest
@AutoConfigureMockMvc
class ValidationTest(
    @Autowired private val mockMvc: MockMvc,
) {
    companion object {
        private val tmpRoot: Path = Files.createTempDirectory("goat-validation")

        @JvmStatic
        @DynamicPropertySource
        fun properties(registry: DynamicPropertyRegistry) {
            registry.add("app.packer-bin") { FakeBins.install(tmpRoot, "fake-packer.sh") }
            registry.add("app.scheduler-bin") { FakeBins.install(tmpRoot, "fake-scheduler.sh") }
            registry.add("app.profiles-dir") { "../../profiles" }
            registry.add("app.jobs-root") { tmpRoot.resolve("jobs").toString() }
        }
    }

    private val goat = MockMultipartFile("file", "goat.3mf", MediaType.APPLICATION_OCTET_STREAM_VALUE, miniProject3mf("Bambu Lab A2L"))

    @Test
    fun `pack без файла — 400`() {
        mockMvc.perform(multipart("/api/pack/jobs").param("profileId", "bambu_a2l"))
            .andExpect(status().isBadRequest)
    }

    @Test
    fun `pack с clearance вне диапазона — 400`() {
        mockMvc.perform(
            multipart("/api/pack/jobs")
                .file(goat)
                .param("profileId", "bambu_a2l")
                .param("clearance", "100"),
        )
            .andExpect(status().isBadRequest)
    }

    @Test
    fun `pack с несуществующим профилем — 400`() {
        mockMvc.perform(
            multipart("/api/pack/jobs")
                .file(goat)
                .param("profileId", "bogus_printer"),
        )
            .andExpect(status().isBadRequest)
    }

    @Test
    fun `pack с nozzleClean=абракадабра — 400`() {
        mockMvc.perform(
            multipart("/api/pack/jobs")
                .file(goat)
                .param("profileId", "bambu_p1s")
                .param("nozzleClean", "абракадабра"),
        )
            .andExpect(status().isBadRequest)
    }

    @Test
    fun `pack с nozzleClean на принтере без кода чистки (a2l, u1) — 400`() {
        for (profileId in listOf("bambu_a2l", "snapmaker_u1")) {
            mockMvc.perform(
                multipart("/api/pack/jobs")
                    .file(goat)
                    .param("profileId", profileId)
                    .param("nozzleClean", "global"),
            )
                .andExpect(status().isBadRequest)
        }
    }

    @Test
    fun `pack с окном, сменой или запасом вне диапазона — 400`() {
        for ((name, value) in listOf(
            "timeWindowHours" to "0.4", "timeWindowHours" to "241",
            "changeSeconds" to "-1", "changeSeconds" to "601",
            "timeMarginPct" to "-1", "timeMarginPct" to "51",
        )) {
            mockMvc.perform(
                multipart("/api/pack/jobs")
                    .file(goat)
                    .param("profileId", "bambu_a2l")
                    .param("amsCorrection", "true")
                    .param(name, value),
            )
                .andExpect(status().isBadRequest)
        }
    }

    @Test
    fun `pack и schedule с коррекцией AMS на тулчейнджере (u1) — 400`() {
        val u1 = MockMultipartFile("file", "goat.3mf", MediaType.APPLICATION_OCTET_STREAM_VALUE, miniProject3mf("Snapmaker U1"))
        for (endpoint in listOf("/api/pack/jobs", "/api/schedule/jobs")) {
            val body = mockMvc.perform(
                multipart(endpoint)
                    .file(u1)
                    .param("profileId", "snapmaker_u1")
                    .param("amsCorrection", "true"),
            )
                .andExpect(status().isBadRequest)
                .andReturn().response.contentAsString
            org.junit.jupiter.api.Assertions.assertTrue(body.contains("коррекция"), body)
        }
    }

    @Test
    fun `schedule со сменой прутка вне диапазона — 400`() {
        mockMvc.perform(
            multipart("/api/schedule/jobs")
                .file(goat)
                .param("profileId", "bambu_a2l")
                .param("amsCorrection", "true")
                .param("changeSeconds", "700"),
        )
            .andExpect(status().isBadRequest)
    }

    @Test
    fun `pack с файлом не 3mf-расширения — 400`() {
        val stl = MockMultipartFile("file", "goat.stl", MediaType.APPLICATION_OCTET_STREAM_VALUE, miniProject3mf("Bambu Lab A2L"))
        mockMvc.perform(
            multipart("/api/pack/jobs")
                .file(stl)
                .param("profileId", "bambu_a2l"),
        )
            .andExpect(status().isBadRequest)
    }

    @Test
    fun `schedule с dayStart позже nightStart — 400`() {
        mockMvc.perform(
            multipart("/api/schedule/jobs")
                .file(goat)
                .param("profileId", "bambu_a2l")
                .param("dayStart", "23:00")
                .param("nightStart", "10:00"),
        )
            .andExpect(status().isBadRequest)
    }

    @Test
    fun `schedule с horizonEnd больше 47 часов — 400`() {
        mockMvc.perform(
            multipart("/api/schedule/jobs")
                .file(goat)
                .param("profileId", "bambu_a2l")
                .param("horizonEnd", "99:00"),
        )
            .andExpect(status().isBadRequest)
    }

    @Test
    fun `schedule без файла и без packJobId — 400`() {
        mockMvc.perform(multipart("/api/schedule/jobs").param("profileId", "bambu_a2l"))
            .andExpect(status().isBadRequest)
    }

    @Test
    fun `schedule с несуществующим packJobId — 400`() {
        mockMvc.perform(
            multipart("/api/schedule/jobs")
                .param("packJobId", "no-such-job")
                .param("profileId", "bambu_a2l"),
        )
            .andExpect(status().isBadRequest)
    }
}
