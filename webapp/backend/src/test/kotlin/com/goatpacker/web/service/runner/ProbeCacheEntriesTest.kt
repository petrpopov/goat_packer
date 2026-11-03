package com.goatpacker.web.service.runner

import com.fasterxml.jackson.databind.ObjectMapper
import org.junit.jupiter.api.Assertions.assertFalse
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test

class ProbeCacheEntriesTest {
    private val mapper = ObjectMapper()
    private val key = "ab".repeat(32)

    private fun valid(name: String, body: String) = ProbeCacheEntries.isValid(name, body.toByteArray(), mapper)

    @Test fun `only the packer's two file names are entries, nothing with path or shell characters`() {
        assertTrue(ProbeCacheEntries.isEntryName("probe-$key.json"))
        assertTrue(ProbeCacheEntries.isEntryName("support-$key.json"))
        for (name in listOf("probe-$key.json.part", "../probe-$key.json", "probe-${key.uppercase()}.json", "probe-abc.json", "probe-$key.json; rm", "x/probe-$key.json", "input.3mf")) {
            assertFalse(ProbeCacheEntries.isEntryName(name), name)
        }
    }

    @Test fun `a probe must carry its own key, runs and rotation`() {
        val name = "probe-$key.json"
        assertTrue(valid(name, """{"rotation_deg":90.0,"runs":3,"cache_key":"$key","tower":[]}"""))
        assertFalse(valid(name, """{"rotation_deg":90.0,"runs":3,"cache_key":"${"cd".repeat(32)}"}"""))
        assertFalse(valid(name, """{"rotation_deg":90.0,"runs":0,"cache_key":"$key"}"""))
        assertFalse(valid(name, """{"rotation_deg":90.0,"runs":100000,"cache_key":"$key"}"""))
        assertFalse(valid(name, """{"runs":3,"cache_key":"$key"}"""))
        assertFalse(valid(name, """[1,2]"""))
        assertFalse(valid(name, "not json"))
        assertFalse(valid("probe-bad.json", """{"rotation_deg":0,"runs":3,"cache_key":"$key"}"""))
    }

    @Test fun `a support ensemble needs members and realizations arrays`() {
        val name = "support-$key.json"
        assertTrue(valid(name, """{"members":[],"realizations":[]}"""))
        assertFalse(valid(name, """{"members":[]}"""))
        assertFalse(valid(name, """{"members":{},"realizations":[]}"""))
    }
}
