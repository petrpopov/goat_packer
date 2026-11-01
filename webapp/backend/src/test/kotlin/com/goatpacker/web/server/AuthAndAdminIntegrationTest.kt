package com.goatpacker.web.server

import com.goatpacker.web.entity.newId
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertFalse
import org.junit.jupiter.api.Assertions.assertNull
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test
import org.springframework.test.web.servlet.request.MockMvcRequestBuilders.post
import java.security.MessageDigest

/** Сессии, роли, CSRF, админка, сброс пароля, ограничения тела запроса и проверка «не бот». */
class AuthAndAdminIntegrationTest : ServerIntegrationTestBase() {
    @Test fun `server denies legacy endpoints admin to guests and mutation without csrf`() {
        val guest = Client()
        guest.guest()
        guest.call("GET", "/api/v2/admin/users", expected = 403)
        guest.call("GET", "/api/profiles", expected = 403)
        assertEquals(403, mvc.perform(post("/api/v2/auth/guest")).andReturn().response.status)
    }

    @Test fun `a session slides with use and an expired one answers 401 in readable utf-8`() {
        val client = Client()
        val id = client.guest()["id"].asText()
        // Сессия за час до конца (гость зашёл почти неделю назад): обращение продлевает её и переписывает cookie.
        jdbc.update("update auth_sessions set expires_at=now()+interval '1 hour' where owner_id=?", id)
        val renewed = client.raw("GET", "/api/v2/auth/me")
        assertEquals(200, renewed.status)
        assertTrue(
            renewed.getHeaders("Set-Cookie").any { it.startsWith("GP_SESSION=") && it.contains("Max-Age=604800") },
            renewed.getHeaders("Set-Cookie").toString(),
        )
        assertTrue(jdbc.queryForObject("select expires_at>now()+interval '6 days' from auth_sessions where owner_id=?", Boolean::class.java, id)!!)
        // Свежая сессия cookie не трогает: продление не чаще раза в сутки.
        assertTrue(client.raw("GET", "/api/v2/auth/me").getHeaders("Set-Cookie").none { it.startsWith("GP_SESSION=") })
        // Истёкшая — 401 с русским текстом в UTF-8 (без кодировки сервлет писал ISO-8859-1, браузер показывал «????»).
        jdbc.update("update auth_sessions set expires_at=now()-interval '1 minute' where owner_id=?", id)
        val expired = client.raw("POST", "/api/v2/jobs/${newId()}/check")
        assertEquals(401, expired.status)
        assertEquals("UTF-8", expired.characterEncoding.uppercase())
        assertTrue(expired.getContentAsString(Charsets.UTF_8).contains("Сессия истекла"), expired.getContentAsString(Charsets.UTF_8))
    }

    @Test fun `administrator sees guests blocks access and cannot block self`() {
        val guest = Client()
        val id = guest.guest()["id"].asText()
        val admin = Client()
        val adminId = admin.login()["id"].asText()
        assertTrue(admin.call("GET", "/api/v2/admin/users?kind=GUEST")["items"].any { it["id"].asText() == id })
        admin.call("POST", "/api/v2/admin/users/$id/block", mapOf("blocked" to true, "reason" to "abuse report"))
        guest.call("GET", "/api/v2/auth/me", expected = 403)
        admin.call("POST", "/api/v2/admin/users/$adminId/block", mapOf("blocked" to true, "reason" to "test guard"), 400)
        assertTrue(admin.call("GET", "/api/v2/admin/audit").any { it["action"].asText() == "ACCESS_BLOCKED" })
    }

    @Test fun `password reset revokes sessions cannot be reused and never reveals password`() {
        val user = Client()
        user.guest()
        val name = "user${nextNumber()}"
        val registration = mapOf("login" to name, "password" to "Original-password-123", "role" to "ADMIN")
        val id = user.call("POST", "/api/v2/auth/register", registration)["id"].asText()
        assertEquals("USER", user.call("GET", "/api/v2/auth/me")["role"].asText())
        val admin = Client()
        admin.login()
        val response = admin.call("POST", "/api/v2/admin/users/$id/password-reset", mapOf("reason" to "User requested reset"))
        user.call("GET", "/api/v2/auth/me", expected = 401)
        val token = response["token"].asText()
        assertTrue(token.length >= 40)
        user.call("POST", "/api/v2/auth/reset", mapOf("token" to token, "password" to "Replacement-secret-123"))
        user.call("POST", "/api/v2/auth/reset", mapOf("token" to token, "password" to "Replacement-secret-456"), 400)
        user.login(name, "Replacement-secret-123")
        assertFalse(admin.call("GET", "/api/v2/admin/users?query=$name").toString().contains("passwordHash"))
    }

    @Test fun `admin system limits pause admission without pretending to kill running jobs`() {
        val admin = Client()
        admin.login()
        admin.call("POST", "/api/v2/admin/system", mapOf("maxParallel" to 0, "paused" to true, "reason" to "maintenance"))
        val state = admin.call("GET", "/api/v2/admin/system")
        assertEquals(0, state["configuredParallelism"].asInt())
        assertTrue(state["paused"].asBoolean())
        assertNull(execution.claim("none"))
    }

    @Test fun `proof is owner bound one use and required before bytes are admitted`() {
        val oldRequire = props.requireChallenge
        val oldDifficulty = props.powDifficulty
        props.requireChallenge = true
        props.powDifficulty = 8
        try {
            val a = Client()
            a.guest()
            val b = Client()
            b.guest()
            val challenge = a.call("POST", "/api/v2/challenges")
            var nonce = 0L
            val hash = MessageDigest.getInstance("SHA-256")
            while (hash.digest("${challenge["id"].asText()}:${challenge["salt"].asText()}:$nonce".toByteArray())[0].toInt() != 0) nonce++
            val intent = mapOf(
                "name" to "test.3mf",
                "sizeBytes" to 100,
                "challengeId" to challenge["id"].asText(),
                "nonce" to nonce.toString(),
            )
            a.call("POST", "/api/v2/uploads", mapOf("name" to "test.3mf", "sizeBytes" to 100), 400)
            b.call("POST", "/api/v2/uploads", intent, 400)
            val upload = a.call("POST", "/api/v2/uploads", intent)["fileId"].asText()
            a.call("DELETE", "/api/v2/uploads/$upload")
            a.call("POST", "/api/v2/uploads", intent, 400)
        } finally {
            props.requireChallenge = oldRequire
            props.powDifficulty = oldDifficulty
        }
    }

    @Test fun `control plane rejects oversized json before allocating an object tree`() {
        val response = mvc.perform(post("/api/v2/auth/login").contentType("application/json").content("x".repeat(65537))).andReturn().response
        assertEquals(413, response.status)
    }
}
