package com.goatpacker.web.server

import com.goatpacker.web.FakeBins
import com.goatpacker.web.descriptorProject3mf
import com.goatpacker.web.entity.QueueStatus
import com.goatpacker.web.entity.newId
import com.goatpacker.web.service.CustomProfile
import com.goatpacker.web.service.StlInputs
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertFalse
import org.junit.jupiter.api.Assertions.assertNotEquals
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test
import java.nio.file.Files

/** Полный путь задачи через исполнителя на фейковых бинарниках: упаковка, STL, свой стол, проверка нарезкой, калибровка. */
class JobExecutionIntegrationTest : ServerIntegrationTestBase() {
    private val activeStateNames = QueueStatus.ACTIVE.map { it.name }

    @Test fun `stl packs on any catalog printer or a custom bed, converted by source_project before the packer, and schedules from the packed project`() {
        val c = Client()
        c.guest()
        assertEquals("Нужен файл .3mf или .stl", c.call("POST", "/api/v2/uploads", mapOf("name" to "cat.obj", "sizeBytes" to 10), 400)["message"].asText())
        val broken = c.call("POST", "/api/v2/uploads", mapOf("name" to "broken.stl", "sizeBytes" to 10))["fileId"].asText()
        val brokenUpload = c.call("PUT", "/api/v2/uploads/$broken/content", "PK34567890".toByteArray(), 400, mapOf("Origin" to ORIGIN))
        assertEquals(StlInputs.INVALID_STL, brokenUpload["message"].asText())
        val stl = c.uploadRaw("cat.stl", "solid cat\nfacet normal 0 0 1\nendsolid cat\n".toByteArray())
        assertEquals("READY", stl["state"].asText())
        assertEquals("stl", stl["kind"].asText())
        assertTrue(stl["profileId"].isNull, stl.toString())
        val file = stl["fileId"].asText()
        val calibrationOfStl = c.send(mapOf("fileId" to file, "profileId" to "bambu_p1s", "kind" to "CALIBRATION"), 400)
        assertEquals("STL можно только упаковать: для расписания и калибровки нужен проект .3mf", calibrationOfStl["message"].asText())
        // Локальный раннер сервера без GOAT_SLICER_PRESETS: отказ при постановке, а не в воркере.
        assertEquals(StlInputs.LOCAL_PRESETS_MISSING, c.send(mapOf("fileId" to file, "profileId" to "prusa_mk4s"), 400)["message"].asText())

        val presets = Files.createDirectories(sharedRoot().resolve("orca-profiles"))
        val saved = listOf(app.slicerPresets, app.sourceProjectBin, app.orcaSlicer)
        app.slicerPresets = presets.toString()
        app.orcaSlicer = "/fake/orca"
        app.sourceProjectBin = FakeBins.install(sharedRoot(), "fake-source-project.sh")
        app.resolveAbsolutePaths()
        try {
            withWorker { worker ->
                val catalog = c.send(mapOf("fileId" to file, "profileId" to "prusa_mk4s"), 200)["jobId"].asText()
                val bed = mapOf("width" to 220, "depth" to 220, "height" to 250)
                val custom = c.send(mapOf("fileId" to file, "customBed" to bed), 200)["jobId"].asText()
                assertEquals("Custom Printer", json.readTree(jobs.findById(custom).orElseThrow().profileJson)["printer_name"].asText())
                for (id in listOf(catalog, custom)) {
                    worker.runUntilSettled(id)
                    val response = c.call("GET", "/api/v2/jobs/$id")
                    assertEquals("SUCCEEDED", response["status"].asText(), response.toString())
                    assertTrue(response["artifacts"][0]["name"].asText().startsWith("cat_"), response.toString())
                }
                // «В расписание» после упаковки STL: источник — packed.3mf, т. е. проект. Имя становится .3mf, и
                // воркер кладёт его как input.3mf (раньше — input.stl, и daily_scheduler не находил вход).
                // Профиль каталога несёт слайсер образа: локальный раннер подменяет его (иначе фейк отказывает).
                val schedule = c.send(mapOf("kind" to "SCHEDULE", "packJobId" to catalog, "profileId" to "prusa_mk4s"), 200)["jobId"].asText()
                assertEquals("cat.3mf", jobs.findById(schedule).orElseThrow().originalName)
                worker.runUntilSettled(schedule)
                val response = c.call("GET", "/api/v2/jobs/$schedule")
                assertEquals("SUCCEEDED", response["status"].asText(), response.toString())
                assertTrue(
                    response["artifacts"].any { it["name"].asText().startsWith("cat_prusa_mk4s_") && it["name"].asText().endsWith("_schedule.3mf") },
                    response.toString(),
                )
            }
        } finally {
            app.slicerPresets = saved[0]
            app.sourceProjectBin = saved[1]
            app.orcaSlicer = saved[2]
            app.resolveAbsolutePaths()
        }
    }

    @Test fun `custom bed job packs an unknown printer 3MF with an ad-hoc profile from the confirmed project bed`() {
        val c = Client()
        c.guest()
        val settings = mapOf(
            "printable_area" to listOf("-5x0", "245x0", "245x210", "-5x210"),
            "bed_exclude_area" to listOf("0x0"),
            "printable_height" to "220",
        )
        val file = c.uploadRaw("cat.3mf", descriptorProject3mf("Acme Unknown 3000", settings))["fileId"].asText()
        val bed = mapOf("width" to 250, "depth" to 210, "height" to 220)
        c.send(mapOf("fileId" to file, "profileId" to "bambu_p1s", "customBed" to bed), 400)
        c.send(mapOf("fileId" to file), 400)
        c.send(mapOf("fileId" to file, "kind" to "SCHEDULE", "customBed" to bed), 400)
        c.send(mapOf("fileId" to file, "customBed" to bed + ("height" to 1001)), 400)
        c.send(mapOf("fileId" to file, "customBed" to bed + ("margin" to 31)), 400)
        // Слайсер режет 3MF по столу проекта: размеры менять нельзя, только отступ.
        val resized = c.send(mapOf("fileId" to file, "customBed" to bed + ("width" to 300)), 400)
        assertEquals(CustomProfile.BED_FROM_PROJECT, resized["message"].asText())
        // Распознанный принтер — только своим профилем каталога.
        val p1s = c.uploadRaw("p1s.3mf", descriptorProject3mf("Bambu Lab P1S"))["fileId"].asText()
        val recognised = c.send(mapOf("fileId" to p1s, "customBed" to mapOf("width" to 256, "depth" to 256, "height" to 250)), 400)
        assertEquals(CustomProfile.RECOGNISED_PRINTER, recognised["message"].asText())

        val id = c.send(mapOf("fileId" to file, "customBed" to bed + ("margin" to 7)), 200)["jobId"].asText()
        val record = jobs.findById(id).orElseThrow()
        assertEquals("custom", record.profileId)
        val profile = json.readTree(record.profileJson)
        // Стол — проекта: его начало координат и исключения; отступ — введённый.
        assertEquals(listOf(-5.0, 0.0, 245.0, 210.0), profile["printable_area"].map { it.asDouble() })
        assertEquals(7.0, profile["edge_margin_mm"].asDouble(), 1e-9)
        val status = c.call("GET", "/api/v2/jobs/$id")
        assertEquals("custom", status["profile"]["id"].asText())
        assertEquals(250.0, status["profile"]["bedWidthMm"].asDouble(), 1e-9)
        assertEquals(210.0, status["profile"]["bedDepthMm"].asDouble(), 1e-9)
        assertEquals(7.0, status["profile"]["edgeMarginMm"].asDouble(), 1e-9)
        assertEquals(7.0, status["params"]["customBed"]["margin"].asDouble(), 1e-9)
        assertEquals(250.0, status["params"]["customBed"]["width"].asDouble(), 1e-9)
        assertEquals("Acme Unknown 3000", profile["printer_name"].asText())
        assertEquals("orca", profile["slicer_kind"].asText())
        withWorker { worker ->
            worker.runUntilSettled(id)
            val response = c.call("GET", "/api/v2/jobs/$id")
            assertEquals("SUCCEEDED", response["status"].asText(), response.toString())
            assertTrue(response["artifacts"][0]["name"].asText().contains("_custom_"), response.toString())
        }
    }

    @Test fun `complete durable job workflow publishes named files and hides diagnostics`() {
        val client = Client()
        client.guest()
        val file = client.upload()
        // Окно/смена/запас вне диапазона отсекаются при постановке, до очереди.
        for (invalid in listOf(mapOf("timeWindowHours" to 0.1), mapOf("changeSeconds" to 601), mapOf("timeMarginPct" to 51))) {
            client.job(file, extra = invalid, expected = 400)
        }
        val settings = mapOf(
            "timeWindowHours" to 12,
            "amsCorrection" to true,
            "changeSeconds" to 80,
            "timeMarginPct" to 15,
            "supportOverlapMm" to 1.5,
        )
        val id = client.job(file, extra = settings)["jobId"].asText()
        withWorker { worker ->
            worker.runUntilSettled(id)
            val response = client.call("GET", "/api/v2/jobs/$id")
            assertEquals("SUCCEEDED", response["status"].asText(), response.toString())
            // Фейковый пакер с окном пишет стол «12 из 30»; блок time доходит до ответа сервера.
            assertEquals(12, response["pack"]["objectCount"].asInt())
            assertEquals(12.0, response["pack"]["time"]["windowHours"].asDouble(), 1e-9)
            assertEquals(30, response["pack"]["time"]["fullCount"].asInt())
            // --support-overlap-mm доходит до пакера; поля поддержек отчёта — до ответа.
            assertTrue(response["pack"]["supportsDetected"].asBoolean(), response.toString())
            assertEquals(1.5, response["pack"]["supportOverlapMm"].asDouble(), 1e-9)
            // layout-событие пакера (local runner ставит GOAT_LOG_FORMAT=json) живёт и после успеха.
            assertEquals(2, response["layout"]["placements"].size(), response.toString())
            // Список не тащит раскладку (до десятков КБ на строку при опросе раз в 5 с); одиночный статус — тащит.
            val listed = client.call("GET", "/api/v2/jobs")["items"].first { it["jobId"].asText() == id }
            assertTrue(listed["layout"] == null || listed["layout"].isNull, listed.toString())
            assertEquals(1, response["artifacts"].size())
            assertTrue(response["artifacts"][0]["name"].asText().startsWith("cat_"))
            assertTrue(client.call("GET", "/api/v2/jobs/$id?debug=true")["artifacts"].size() > 1)
            client.call("DELETE", "/api/v2/jobs/$id")
            client.call("GET", "/api/v2/jobs/$id", expected = 404)
        }
    }

    @Test fun `pack without the final slice keeps its state and the finish job slices it`() {
        val client = Client()
        client.guest()
        val file = client.upload()
        // Блоб состояния ставит только сервер.
        client.job(file, extra = mapOf("finishStateBlobId" to "x"), expected = 400)
        val pack = client.job(file, extra = mapOf("finalSlice" to false))["jobId"].asText()
        withWorker { worker ->
            worker.runUntilSettled(pack)
            val packed = client.call("GET", "/api/v2/jobs/$pack?debug=true")
            assertEquals("SUCCEEDED", packed["status"].asText(), packed.toString())
            assertEquals(false, packed["pack"]["finalSliced"].asBoolean(), packed.toString())
            assertTrue(packed["artifacts"].any { it["name"].asText() == "pack_state.json" }, packed.toString())
            // Проверка нарезкой — внутри той же задачи: ответ с id родителя, ребёнок скрыт; фейковый пакер падает, если состояния нет рядом.
            val key = newId()
            val answered = client.call("POST", "/api/v2/jobs/$pack/check", headers = mapOf("Idempotency-Key" to key))["jobId"].asText()
            assertEquals(pack, answered)
            // Повтор того же ключа не создаёт второго ребёнка.
            assertEquals(pack, client.call("POST", "/api/v2/jobs/$pack/check", headers = mapOf("Idempotency-Key" to key))["jobId"].asText())
            val child = jobs.findById(pack).orElseThrow().checkJobId!!
            assertEquals(pack, jobs.findById(child).orElseThrow().parentJobId)
            assertEquals(1, jdbc.queryForObject("select count(*) from queue_jobs where parent_job_id=?", Int::class.java, pack))
            // Пока проверка не кончилась: в статусе родителя она в очереди/работе, второй запуск отклоняется.
            assertTrue(client.call("GET", "/api/v2/jobs/$pack")["check"]["status"].asText() in activeStateNames)
            client.call("POST", "/api/v2/jobs/$pack/check", expected = 400, headers = mapOf("Idempotency-Key" to newId()))
            worker.runUntilSettled(child)
            val finished = client.call("GET", "/api/v2/jobs/$pack?debug=true")
            val check = finished["check"]
            assertEquals("SUCCEEDED", check["status"].asText(), finished.toString())
            assertEquals(true, check["pack"]["finalSliced"].asBoolean(), finished.toString())
            assertFalse(finished.toString().contains(child), "id ребёнка не должен попадать в ответ")
            // Родитель остаётся собой; артефакты проверки скачиваются по id родителя.
            assertEquals(false, finished["pack"]["finalSliced"].asBoolean())
            assertTrue(check["artifacts"].all { it["downloadUrl"].asText().startsWith("/api/v2/jobs/$pack/artifacts/") }, check.toString())
            assertTrue(check["artifacts"].none { it["name"].asText() == "pack_state.json" }, check.toString())
            val childArtifact = check["artifacts"].first { it["name"].asText().endsWith(".3mf") }
            assertTrue(client.call("POST", childArtifact["downloadUrl"].asText(), emptyMap<String, String>())["url"].asText().isNotBlank())
            assertTrue(client.bytes("/api/v2/jobs/$pack/artifacts/${childArtifact["id"].asText()}/file").isNotEmpty())
            // Чужой задачей (другой упаковкой того же пользователя) этот артефакт не отдаётся.
            val other = client.job(file, extra = mapOf("finalSlice" to false))["jobId"].asText()
            client.call("POST", "/api/v2/jobs/$other/artifacts/${childArtifact["id"].asText()}/download", emptyMap<String, String>(), 404)
            client.bytes("/api/v2/jobs/$other/artifacts/${childArtifact["id"].asText()}/file", 404)
            // Ребёнок скрыт: в списке одна строка на упаковку, по его id статус недоступен.
            val listed = client.call("GET", "/api/v2/jobs")
            assertEquals(2, listed["total"].asInt(), listed.toString())
            assertTrue(listed["items"].none { it["jobId"].asText() == child })
            assertTrue(listed["items"].first { it["jobId"].asText() == pack }["check"]["status"].asText() == "SUCCEEDED")
            client.call("GET", "/api/v2/jobs/$child", expected = 404)
            // Суточный лимит гостя (4) проверка не тратит: посчитана упаковка (+ «other»).
            val owner = jobs.findById(pack).orElseThrow().ownerId
            assertEquals(2, jdbc.queryForObject("select used from request_limits where key=?", Int::class.java, "jobs:$owner"))
            // Стол уже проверен нарезкой; через старый путь packJobId — то же самое.
            client.call("POST", "/api/v2/jobs/$pack/check", expected = 400, headers = mapOf("Idempotency-Key" to newId()))
            client.call("POST", "/api/v2/jobs", mapOf("kind" to "PACK", "packJobId" to pack), 400, mapOf("Idempotency-Key" to newId()))
            // Удаление упаковки уносит ребёнка, а его артефакты уходят в GC.
            val keys = jdbc.queryForList("select object_key from artifacts where job_id=?", String::class.java, child)
            assertTrue(keys.isNotEmpty())
            client.call("DELETE", "/api/v2/jobs/$pack")
            assertEquals(0, jdbc.queryForObject("select count(*) from queue_jobs where id=?", Int::class.java, child))
            keys.forEach { key ->
                assertEquals(1, jdbc.queryForObject("select count(*) from object_gc where object_key=?", Int::class.java, key), key)
            }
        }
    }

    @Test fun `a failed check may be re-run and old clients still get the parent id`() {
        val client = Client()
        client.guest()
        val file = client.upload()
        val pack = client.job(file, extra = mapOf("finalSlice" to false))["jobId"].asText()
        withWorker { worker ->
            worker.runUntilSettled(pack)
            // Чужая задача проверять не может.
            val stranger = Client()
            stranger.guest()
            stranger.call("POST", "/api/v2/jobs/$pack/check", expected = 404, headers = mapOf("Idempotency-Key" to newId()))
            // Старый путь: POST /jobs с packJobId отвечает id родителя.
            val legacy = client.call("POST", "/api/v2/jobs", mapOf("kind" to "PACK", "packJobId" to pack), headers = mapOf("Idempotency-Key" to newId()))
            assertEquals(pack, legacy["jobId"].asText())
            val first = jobs.findById(pack).orElseThrow().checkJobId!!
            // Отмена очереди: проверка CANCELLED, её можно запустить снова — ссылка заменяется.
            client.call("POST", "/api/v2/jobs/$pack/check/cancel")
            assertEquals("CANCELLED", client.call("GET", "/api/v2/jobs/$pack")["check"]["status"].asText())
            client.call("POST", "/api/v2/jobs/$pack/check", headers = mapOf("Idempotency-Key" to newId()))
            val second = jobs.findById(pack).orElseThrow().checkJobId!!
            assertNotEquals(first, second)
            assertEquals("QUEUED", client.call("GET", "/api/v2/jobs/$pack")["check"]["status"].asText())
            // Ожидающий в очереди ребёнок отменяется и уходит каскадом вместе с упаковкой.
            client.call("DELETE", "/api/v2/jobs/$pack")
            assertEquals(0, jdbc.queryForObject("select count(*) from queue_jobs where id in (?,?,?)", Int::class.java, pack, first, second))
        }
    }

    @Test fun `calibration job runs on the worker and publishes test files and values`() {
        val client = Client()
        client.guest()
        val file = client.upload()
        client.job(file, extra = mapOf("kind" to "CALIBRATION", "variant" to "extreme"), expected = 400)
        client.job(file, extra = mapOf("supportOverlapMm" to 21), expected = 400)
        val id = client.job(file, extra = mapOf("kind" to "CALIBRATION", "variant" to "perfectionist"))["jobId"].asText()
        withWorker { worker ->
            worker.runUntilSettled(id)
            val response = client.call("GET", "/api/v2/jobs/$id")
            assertEquals("SUCCEEDED", response["status"].asText(), response.toString())
            assertEquals("calibration", response["kind"].asText())
            val calibration = response["calibration"]
            assertEquals(30, calibration["changesExpected"].asInt())
            assertEquals(1500.0, calibration["estimateASeconds"].asDouble(), 1e-9)
            assertEquals(2400.0, calibration["estimateBSeconds"].asDouble(), 1e-9)
            assertEquals(20.5, calibration["slicerChangeSeconds"].asDouble(), 1e-9)
            assertTrue(calibration["instructions"].asText().contains("calib_B.3mf"))
            assertTrue(response["pack"].isNull)
            // Файлы теста — под своими именами (на них ссылается инструкция); журнал — только в debug.
            val names = response["artifacts"].map { it["name"].asText() }.toSet()
            assertEquals(setOf("calib_A.3mf", "calib_B.3mf", "instructions.md", "calibration.json"), names)
        }
    }
}
