package com.goatpacker.web.server

import com.goatpacker.web.descriptorProject3mf
import com.goatpacker.web.repository.ProbeCacheRepository
import com.goatpacker.web.service.server.QueueWorker
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test
import org.springframework.beans.factory.annotation.Autowired
import java.nio.file.Files

/** Кэш проб воркера на фейковом пакере: вторая джоба той же модели видит запись, чужой владелец и подделка — нет. */
class ProbeCacheIntegrationTest : ServerIntegrationTestBase() {
    @Autowired private lateinit var entries: ProbeCacheRepository

    private fun cacheVerdict(jobId: String): String {
        val key = jdbc.queryForObject("select object_key from artifacts where job_id=? and name='placements.json'", String::class.java, jobId)!!
        val file = Files.createTempFile("placements", ".json")
        store.get(key, file)
        return json.readTree(Files.readString(file))["cache"].asText().also { Files.delete(file) }
    }

    private fun packOnce(worker: QueueWorker, client: Client, file: String): String {
        val id = client.job(file)["jobId"].asText()
        worker.runUntilSettled(id)
        assertEquals("SUCCEEDED", client.call("GET", "/api/v2/jobs/$id")["status"].asText())
        return id
    }

    @Test fun `the second job of the same model and owner reuses the cache entry, a forged entry is dropped, another owner starts cold`() {
        val owner = Client()
        val ownerId = owner.guest()["id"].asText()
        val file = owner.upload()
        val stranger = Client()
        val strangerId = stranger.guest()["id"].asText()
        val strangerFile = stranger.upload()
        withWorker { worker ->
            assertEquals("miss", cacheVerdict(packOnce(worker, owner, file)))
            val rows = jdbc.queryForList("select name,object_key,owner_id from probe_cache_entries")
            assertEquals(1, rows.size, rows.toString())
            assertTrue(rows[0]["name"].toString().startsWith("probe-"))
            assertTrue(rows[0]["object_key"].toString().startsWith("probe-cache/$ownerId/"), rows.toString())
            assertEquals(ownerId, rows[0]["owner_id"])

            assertEquals("hit", cacheVerdict(packOnce(worker, owner, file)))
            assertEquals(1, jdbc.queryForObject("select count(*) from probe_cache_entries", Int::class.java))

            // Тот же файл у другого владельца: его кэш пуст, запись первого ему не отдаётся.
            assertEquals("miss", cacheVerdict(packOnce(worker, stranger, strangerFile)))
            assertEquals(
                listOf(1, 1),
                listOf(ownerId, strangerId).map { jdbc.queryForObject("select count(*) from probe_cache_entries where owner_id=?", Int::class.java, it) },
            )
        }
    }

    @Test fun `a re-saved copy of the project (other metadata and thumbnail, same mesh) hits the same group, another mesh does not`() {
        val client = Client()
        client.guest()
        val original = client.upload(data = descriptorProject3mf("Test Corner Bed", model = meshModel("2026-10-31", "1.5")))
        val resaved = client.upload(
            data = descriptorProject3mf(
                "Test Corner Bed",
                model = meshModel("2026-11-03", "1.5", uuid = "00040000"),
                extraEntries = mapOf("Metadata/plate_1.png" to "another thumbnail", "Metadata/model_settings.config" to "<config/>"),
            ),
        )
        val edited = client.upload(data = descriptorProject3mf("Test Corner Bed", model = meshModel("2026-10-31", "1.75")))
        withWorker { worker ->
            assertEquals("miss", cacheVerdict(packOnce(worker, client, original)))
            assertEquals("hit", cacheVerdict(packOnce(worker, client, resaved)))
            assertEquals("miss", cacheVerdict(packOnce(worker, client, edited)))
            assertEquals(2, jdbc.queryForObject("select count(distinct group_key) from probe_cache_entries", Int::class.java))
        }
    }

    private fun meshModel(date: String, z: String, uuid: String = "00010000") = """<?xml version="1.0"?>
<model xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02" xmlns:p="http://schemas.microsoft.com/3dmanufacturing/production/2015/06">
 <metadata name="ModificationDate">$date</metadata>
 <resources><object id="1" p:UUID="$uuid-81cb" type="model"><mesh>
  <vertices><vertex x="0" y="0" z="0"/><vertex x="10" y="0" z="0"/><vertex x="0" y="10" z="$z"/></vertices>
  <triangles><triangle v1="0" v2="1" v3="2" paint_color="8"/></triangles>
 </mesh></object></resources>
 <build><item objectid="1" transform="1 0 0 0 1 0 0 0 1 $date 0 0"/></build>
</model>"""

    @Test fun `with the cache off no entry is stored and the packer gets no cache dir`() {
        props.probeCacheEnabled = false
        try {
            val client = Client()
            client.guest()
            val file = client.upload()
            withWorker { worker ->
                assertEquals("none", cacheVerdict(packOnce(worker, client, file)))
                assertEquals(0, jdbc.queryForObject("select count(*) from probe_cache_entries", Int::class.java))
            }
        } finally {
            props.probeCacheEnabled = true
        }
    }

    @Test fun `eviction keeps the group and the owner within limits and sends objects to gc, expiry drops unused entries`() {
        jdbc.update("delete from probe_cache_entries")
        for (i in 1..3) {
            jdbc.update(
                "insert into probe_cache_entries values ('o','g',?,?,100,now(),now()-make_interval(mins => ?))",
                "probe-$i.json", "probe-cache/o/g/$i/probe-$i.json", i,
            )
        }
        assertEquals(1, entries.evictGroup("o", "g", 2))
        assertEquals(listOf("probe-1.json", "probe-2.json"), jdbc.queryForList("select name from probe_cache_entries order by name", String::class.java))
        assertEquals(1, jdbc.queryForObject("select count(*) from object_gc where object_key='probe-cache/o/g/3/probe-3.json'", Int::class.java))
        jdbc.update("update probe_cache_entries set last_used_at=now()-interval '40 days' where name='probe-2.json'")
        probeCache.expire()
        assertEquals(listOf("probe-1.json"), jdbc.queryForList("select name from probe_cache_entries", String::class.java))
        // GC не трогает объект, на который есть строка.
        jdbc.update("insert into object_gc(object_key,size_bytes,not_before) values ('probe-cache/o/g/1/probe-1.json',100,now()-interval '1 hour')")
        assertTrue(gc.claimDeletions(100).none { it.contains("/1/probe-1") })
    }

}
