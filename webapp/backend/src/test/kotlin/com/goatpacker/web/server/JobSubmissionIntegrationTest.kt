package com.goatpacker.web.server

import com.goatpacker.web.descriptorProject3mf
import com.goatpacker.web.entity.newId
import org.junit.jupiter.api.Assertions.assertArrayEquals
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertFalse
import org.junit.jupiter.api.Assertions.assertNotNull
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test

/** Постановка задач: лимиты, идемпотентность, права доступа, снимок профиля и статус для рабочей области. */
class JobSubmissionIntegrationTest : ServerIntegrationTestBase() {
    @Test fun `admission limits switched off let a guest run past the daily and active job limits`() {
        props.admissionLimits = false
        try {
            val client = Client()
            client.guest()
            val file = client.upload()
            // Гостю обычно 4 задачи в сутки и 2 активных: без лимитов проходят все шесть.
            repeat(6) { client.job(file) }
        } finally {
            props.admissionLimits = true
        }
        val other = Client()
        other.guest()
        val file = other.upload()
        repeat(2) { other.job(file) }
        other.job(file, expected = 429)
    }

    @Test fun `dedup queue ownership idempotency capabilities and profile snapshot`() {
        val a = Client()
        a.guest()
        val b = Client()
        b.guest()
        val file = a.upload()
        b.upload()
        assertEquals(1, jdbc.queryForObject("select count(*) from blobs", Int::class.java))
        val key = newId()
        val created = a.job(file, key)
        val id = created["jobId"].asText()
        assertEquals(id, a.job(file, key)["jobId"].asText())
        assertEquals(1, jdbc.queryForObject("select count(*) from queue_jobs", Int::class.java))
        b.call("GET", "/api/v2/jobs/$id", expected = 404)
        b.call("GET", "/api/v2/jobs/$id", headers = mapOf("X-Job-Token" to created["readToken"].asText()))
        b.call("DELETE", "/api/v2/jobs/$id", expected = 404)
        assertEquals(1, a.call("GET", "/api/v2/jobs/$id")["queuePosition"].asInt())
        assertTrue(jobs.findById(id).orElseThrow().profileJson.contains("Test Corner Bed"))
        a.call("DELETE", "/api/v2/jobs/$id")
        assertEquals("CANCELLED", a.call("GET", "/api/v2/jobs/$id")["status"].asText())
    }

    @Test fun `unknown printer upload carries the project bed for the custom-bed form`() {
        val c = Client()
        c.guest()
        val settings = mapOf(
            "printable_area" to listOf("0x0", "250x0", "250x210", "0x210"),
            "bed_exclude_area" to listOf("0x0", "20x0", "20x30", "0x30"),
            "printable_height" to "220",
        )
        val unknown = c.uploadRaw("cat.3mf", descriptorProject3mf("Acme Unknown 3000", settings))
        assertTrue(unknown["profileId"].isNull)
        assertEquals("3mf", unknown["kind"].asText())
        assertEquals(listOf(0.0, 0.0, 250.0, 210.0), unknown["projectBed"]["printableArea"].map { it.asDouble() })
        assertEquals(listOf(listOf(0.0, 0.0, 20.0, 30.0)), unknown["projectBed"]["exclusionRectangles"].map { rect -> rect.map { it.asDouble() } })
        assertEquals(unknown["projectBed"], c.call("GET", "/api/v2/uploads/${unknown["fileId"].asText()}")["projectBed"])
        assertTrue(c.uploadRaw("corner.3mf", descriptorProject3mf("Test Corner Bed", settings))["projectBed"].isNull)
    }

    @Test fun `pack status carries originalName, profile and params for the read-only workspace, and the input downloads like an artifact`() {
        val a = Client()
        a.guest()
        val b = Client()
        b.guest()
        val data = descriptorProject3mf("Test Corner Bed")
        val file = a.upload()
        val created = a.job(file, extra = mapOf("clearance" to 1.5, "finalSlice" to false))
        val id = created["jobId"].asText()
        val status = a.call("GET", "/api/v2/jobs/$id")
        assertEquals("cat.3mf", status["originalName"].asText())
        assertEquals("test_corner_bed", status["profile"]["id"].asText())
        assertEquals(a.call("GET", "/api/v2/profiles").first { it["id"].asText() == "test_corner_bed" }, status["profile"])
        assertEquals(1.5, status["params"]["clearance"].asDouble(), 1e-9)
        assertFalse(status["params"]["finalSlice"].asBoolean())
        assertEquals("search", status["params"]["layout"].asText())
        assertTrue(status["params"]["customBed"].isNull)
        // Список лёгкий: профиль и настройки только в статусе одной задачи.
        assertFalse(a.call("GET", "/api/v2/jobs")["items"][0].has("profile"))
        val download = a.call("POST", "/api/v2/jobs/$id/input/download")
        assertEquals("cat.3mf", download["filename"].asText())
        assertArrayEquals(data, a.bytes(download["url"].asText()))
        // Без доступа — как для артефактов; по токену чтения — можно.
        b.call("POST", "/api/v2/jobs/$id/input/download", expected = 404)
        b.bytes("/api/v2/jobs/$id/input/file", 404)
        b.call("POST", "/api/v2/jobs/$id/input/download", headers = mapOf("X-Job-Token" to created["readToken"].asText()))
        // Не упаковка: вход не отдаётся, профиль и настройки в статусе null.
        val scheduleId = a.job(file)["jobId"].asText()
        jdbc.update("update queue_jobs set kind='SCHEDULE' where id=?", scheduleId)
        a.call("POST", "/api/v2/jobs/$scheduleId/input/download", expected = 400)
        assertTrue(a.call("GET", "/api/v2/jobs/$scheduleId")["params"].isNull)
    }

    @Test fun `unknown printer 3MF needs a custom bed, the catalog pick is refused`() {
        val c = Client()
        c.guest()
        val file = c.uploadRaw("cat.3mf", descriptorProject3mf("Acme Unknown 3000"))["fileId"].asText()
        assertEquals("Принтер файла не соответствует выбранному профилю", c.job(file, expected = 400)["message"].asText())
    }

    @Test fun `calibration is rejected for a printer without AMS changes`() {
        val client = Client()
        client.guest()
        val file = client.upload("Test Margin Bed", "test_margin_bed")
        val error = client.send(mapOf("fileId" to file, "profileId" to "test_margin_bed", "kind" to "CALIBRATION"), 400)
        assertTrue(error.toString().contains("для этого принтера калибровка не нужна"), error.toString())
    }

    @Test fun `compute reservations enforce global cap and cancellation refunds unused time`() {
        val old = props.globalComputeSeconds
        props.globalComputeSeconds = props.jobTimeoutSeconds
        try {
            val a = Client()
            a.guest()
            val id = a.job(a.upload())["jobId"].asText()
            val b = Client()
            b.guest()
            val file = b.upload()
            b.send(mapOf("fileId" to file, "profileId" to "test_corner_bed"), 429)
            a.call("DELETE", "/api/v2/jobs/$id")
            assertNotNull(b.job(file)["jobId"])
        } finally {
            props.globalComputeSeconds = old
        }
    }
}
