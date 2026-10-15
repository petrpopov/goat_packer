package com.goatpacker.web.server

import com.fasterxml.jackson.databind.ObjectMapper
import org.junit.jupiter.api.Assertions.*
import org.junit.jupiter.api.Test
import org.junit.jupiter.api.io.TempDir
import org.springframework.beans.factory.annotation.Autowired
import org.springframework.boot.test.context.SpringBootTest
import org.springframework.boot.test.web.server.LocalServerPort
import org.springframework.test.context.ActiveProfiles
import org.springframework.test.context.DynamicPropertyRegistry
import org.springframework.test.context.DynamicPropertySource
import org.testcontainers.Testcontainers
import org.testcontainers.containers.GenericContainer
import org.testcontainers.containers.PostgreSQLContainer
import org.testcontainers.utility.MountableFile
import java.net.CookieManager
import java.net.CookiePolicy
import java.net.URI
import java.net.http.HttpClient
import java.net.http.HttpRequest
import java.net.http.HttpResponse
import java.nio.file.Files
import java.nio.file.Path

/** Real Tomcat + real nginx with the deployment config. MockMvc does not run RemoteIpValve,
 * so it cannot catch losing the public port when Docker maps an arbitrary port to nginx:80. */
@SpringBootTest(webEnvironment = SpringBootTest.WebEnvironment.RANDOM_PORT)
@ActiveProfiles("server")
class GatewayOriginIntegrationTest {
    companion object {
        class Pg : PostgreSQLContainer<Pg>("postgres:17.0")
        private val postgres = Pg().apply { start() }
        private val storage = Files.createTempDirectory("goat-gateway-storage-")

        @JvmStatic @DynamicPropertySource
        fun properties(registry: DynamicPropertyRegistry) {
            registry.add("spring.datasource.url", postgres::getJdbcUrl)
            registry.add("spring.datasource.username", postgres::getUsername)
            registry.add("spring.datasource.password", postgres::getPassword)
            registry.add("platform.storage-mode") { "file" }
            registry.add("platform.local-storage-root") { storage.toString() }
            registry.add("platform.secure-cookies") { false }
            registry.add("platform.worker-enabled") { false }
            registry.add("platform.admin-login") { "" }
            // Testcontainers' port-forward tunnel is our only proxy in this isolated server.
            registry.add("server.tomcat.remoteip.internal-proxies") { ".*" }
            registry.add("app.profiles-dir") { "../../profiles" }
        }
    }

    @LocalServerPort private var port: Int = 0
    @Autowired private lateinit var json: ObjectMapper
    @TempDir lateinit var directory: Path

    private fun withGateway(publicScheme: String, check: (origin: String, mappedOrigin: String, client: HttpClient, csrf: String) -> Unit) {
        Testcontainers.exposeHostPorts(port)
        val config = Files.readString(Path.of("../../deploy/nginx.conf"))
            .replace("\${GOAT_PUBLIC_SCHEME}", publicScheme)
            .replace("http://api:19190", "http://host.testcontainers.internal:$port")
            .replace("http://storage:8333", "http://host.testcontainers.internal:$port")
        val configPath = directory.resolve("nginx.conf")
        Files.writeString(configPath, config)
        class Nginx : GenericContainer<Nginx>("nginx:1.28-alpine")
        Nginx().withExposedPorts(80)
            .withCopyFileToContainer(MountableFile.forHostPath(configPath), "/etc/nginx/conf.d/default.conf")
            .use { gateway ->
                gateway.start()
                val mapped = "${gateway.host}:${gateway.getMappedPort(80)}"
                val cookies = CookieManager(null, CookiePolicy.ACCEPT_ALL)
                val client = HttpClient.newBuilder().cookieHandler(cookies).build()
                val csrfResponse = client.send(
                    HttpRequest.newBuilder(URI("http://$mapped/api/v2/auth/csrf")).GET().build(),
                    HttpResponse.BodyHandlers.ofString(),
                )
                assertEquals(200, csrfResponse.statusCode())
                val csrf = json.readTree(csrfResponse.body())["token"].asText()
                check("${publicScheme.ifEmpty { "http" }}://$mapped", "http://$mapped", client, csrf)
            }
    }

    private fun guest(client: HttpClient, target: String, csrf: String, from: String, includeCsrf: Boolean = true, spoofForwarded: Boolean = false): HttpResponse<String> {
        val request = HttpRequest.newBuilder(URI("$target/api/v2/auth/guest"))
            .header("Origin", from).header("Content-Type", "application/json")
        if (includeCsrf) request.header("X-XSRF-TOKEN", csrf)
        if (spoofForwarded) request
            .header("Forwarded", "host=evil.invalid;proto=https")
            .header("X-Forwarded-Host", "evil.invalid")
            .header("X-Forwarded-Port", "443")
            .header("X-Forwarded-Proto", "https")
        return client.send(request.POST(HttpRequest.BodyPublishers.ofString("{}")).build(), HttpResponse.BodyHandlers.ofString())
    }

    @Test
    fun `mapped gateway port is same origin but hostile origin and missing csrf remain forbidden`() =
        withGateway("") { origin, target, client, csrf ->
            val accepted = guest(client, target, csrf, origin, spoofForwarded = true)
            assertEquals(200, accepted.statusCode(), accepted.body())
            assertEquals("GUEST", json.readTree(accepted.body())["kind"].asText())
            val current = client.send(HttpRequest.newBuilder(URI("$target/api/v2/auth/me")).GET().build(), HttpResponse.BodyHandlers.ofString())
            assertEquals(200, current.statusCode(), current.body())
            assertEquals(json.readTree(accepted.body())["id"], json.readTree(current.body())["id"])
            assertEquals(403, guest(client, target, csrf, "https://evil.invalid", spoofForwarded = true).statusCode())
            val noCsrf = guest(client, target, csrf, origin, includeCsrf = false)
            assertEquals(403, noCsrf.statusCode())
            assertEquals("FORBIDDEN", json.readTree(noCsrf.body())["code"].asText())
        }

    /** Pangolin terminates TLS and talks plain HTTP to the gateway: the public scheme is configuration. */
    @Test
    fun `configured https public scheme accepts the https origin and still rejects plain http`() =
        withGateway("https") { origin, target, client, csrf ->
            assertTrue(origin.startsWith("https://"))
            val accepted = guest(client, target, csrf, origin)
            assertEquals(200, accepted.statusCode(), accepted.body())
            assertEquals(403, guest(client, target, csrf, origin.replaceFirst("https://", "http://")).statusCode())
        }
}
