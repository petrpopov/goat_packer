package com.goatpacker.web.profiles

import com.fasterxml.jackson.databind.ObjectMapper
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test
import org.springframework.beans.factory.annotation.Autowired
import org.springframework.boot.test.autoconfigure.web.servlet.AutoConfigureMockMvc
import org.springframework.boot.test.context.SpringBootTest
import org.springframework.mock.web.MockMultipartFile
import org.springframework.test.context.TestPropertySource
import org.springframework.test.web.servlet.MockMvc
import org.springframework.test.web.servlet.request.MockMvcRequestBuilders.multipart
import org.springframework.test.web.servlet.result.MockMvcResultMatchers.status
import java.io.ByteArrayOutputStream
import java.util.zip.ZipEntry
import java.util.zip.ZipOutputStream

/**
 * POST /api/profiles/detect: printer_model из Metadata/project_settings.config загруженного
 * 3MF сопоставляется с профилями. Мини-3MF собирается в памяти как zip с одним конфигом.
 */
@SpringBootTest
@AutoConfigureMockMvc
@TestPropertySource(properties = ["app.profiles-dir=../../profiles"])
class DetectProfileTest(
    @Autowired private val mockMvc: MockMvc,
) {
    private val json = ObjectMapper()

    private fun mini3mf(printerModel: String?): ByteArray {
        val bytes = ByteArrayOutputStream()
        ZipOutputStream(bytes).use { zip ->
            zip.putNextEntry(ZipEntry("Metadata/project_settings.config"))
            val config = if (printerModel == null) {
                """{"printable_area":["0x0","270x0","270x270","0x270"]}"""
            } else {
                """{"printer_model":"$printerModel","printable_area":["0x0","270x0","270x270","0x270"]}"""
            }
            zip.write(config.toByteArray())
            zip.closeEntry()
        }
        return bytes.toByteArray()
    }

    private fun detect(body: ByteArray): String =
        mockMvc.perform(
            multipart("/api/profiles/detect").file(
                MockMultipartFile("file", "model.3mf", "application/octet-stream", body)
            )
        )
            .andExpect(status().isOk)
            .andReturn()
            .response
            .contentAsString

    @Test
    fun `точное имя принтера из 3mf даёт профиль u1`() {
        val response = json.readTree(detect(mini3mf("Snapmaker U1")))
        assertEquals("snapmaker_u1", response["profileId"].asText())
        assertEquals("Snapmaker U1", response["printerModel"].asText())
    }

    @Test
    fun `имя с дополнением всё равно матчится вхождением`() {
        val response = json.readTree(detect(mini3mf("Bambu Lab P1S 0.4 nozzle")))
        assertEquals("bambu_p1s", response["profileId"].asText())
    }

    @Test
    fun `неизвестный принтер и файл без printer_model дают null`() {
        for (body in listOf(mini3mf("Prusa MK4"), mini3mf(null))) {
            val node = json.readTree(detect(body)).path("profileId")
            assertTrue(node.isMissingNode || node.isNull || node.asText("").isEmpty(), "ожидался null: $node")
        }
    }

    @Test
    fun `неизвестный принтер всё же назван в printerModel — фронт покажет чей файл`() {
        val response = json.readTree(detect(mini3mf("Prusa MK4")))
        assertEquals("Prusa MK4", response["printerModel"].asText())
    }

    @Test
    fun `файл без printer_model отдаёт printerModel null`() {
        val response = json.readTree(detect(mini3mf(null)))
        // non_null-сериализация опускает поле целиком — path() не бросает NPE на отсутствующем.
        val model = response.path("printerModel")
        assertTrue(model.isMissingNode || model.isNull || model.asText("").isEmpty())
    }
    @Test
    fun `slicer stored previews with descriptors do not hide printer settings`() {
        val bytes = com.goatpacker.web.descriptorProject3mf("Bambu Lab P1S")
        org.junit.jupiter.api.assertThrows<java.util.zip.ZipException> {
            java.util.zip.ZipInputStream(bytes.inputStream()).use { it.nextEntry }
        }
        val response = json.readTree(detect(bytes))
        assertEquals("bambu_p1s", response["profileId"].asText())
        assertEquals("Bambu Lab P1S", response["printerModel"].asText())
    }

    @Test
    fun `missing printer has explicit nullable fields`() {
        val response = json.readTree(detect(mini3mf(null)))
        assertTrue(response.has("profileId") && response["profileId"].isNull)
        assertTrue(response.has("printerModel") && response["printerModel"].isNull)
    }

    @Test
    fun `broken archive and missing multipart field return 400 instead of unknown printer`() {
        mockMvc.perform(multipart("/api/profiles/detect")).andExpect(status().isBadRequest)
        mockMvc.perform(multipart("/api/profiles/detect").file(MockMultipartFile("file", "bad.3mf", "application/octet-stream", byteArrayOf(1,2,3))))
            .andExpect(status().isBadRequest)
    }

    @Test
    fun `short generic names cannot silently select a different printer`() {
        assertTrue(json.readTree(detect(mini3mf("Bambu Lab P1")))["profileId"].isNull)
    }

}
