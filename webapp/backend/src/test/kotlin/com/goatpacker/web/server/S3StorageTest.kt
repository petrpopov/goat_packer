package com.goatpacker.web.server

import com.goatpacker.web.config.PlatformProperties
import com.goatpacker.web.repository.S3ObjectStore
import org.junit.jupiter.api.Assertions.assertArrayEquals
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertThrows
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test
import org.junit.jupiter.api.io.TempDir
import org.testcontainers.containers.GenericContainer
import java.net.URI
import java.net.http.HttpClient
import java.net.http.HttpRequest
import java.net.http.HttpResponse
import java.nio.file.Files
import java.nio.file.Path
import java.util.Random
import org.junit.jupiter.api.Tag

/** Testcontainers: off in `gradle test`, run by `containersTest`. */
@Tag("containers")
class S3StorageTest {
    @TempDir lateinit var dir: Path

    private class Seaweed : GenericContainer<Seaweed>("chrislusf/seaweedfs:4.47")

    @Test fun `SeaweedFS supports private object roundtrip signed download and deletion`() {
        Seaweed()
            .withExposedPorts(S3_PORT)
            .withEnv("AWS_ACCESS_KEY_ID", "test-access")
            .withEnv("AWS_SECRET_ACCESS_KEY", "test-secret-password")
            .withEnv("S3_BUCKET", "goat")
            .withCommand("mini", "-dir=/data")
            .use { container ->
                container.start()
                val endpoint = "http://${container.host}:${container.getMappedPort(S3_PORT)}"
                val props = PlatformProperties().apply {
                    s3Endpoint = endpoint
                    s3PublicEndpoint = endpoint
                    s3AccessKey = "test-access"
                    s3SecretKey = "test-secret-password"
                }
                val store = S3ObjectStore(props)
                val source = dir.resolve("in.txt")
                Files.writeString(source, "test content")
                store.put("jobs/test/result", source)
                val out = dir.resolve("out.txt")
                store.get("jobs/test/result", out)
                assertEquals("test content", Files.readString(out))
                for (size in listOf(0, 1, 237, 902, 21932, 65536, 2 * 1024 * 1024)) {
                    val bytes = ByteArray(size).also { Random(17).nextBytes(it) }
                    Files.write(source, bytes)
                    store.put("jobs/test/binary-$size", source)
                    store.get("jobs/test/binary-$size", out)
                    assertArrayEquals(bytes, Files.readAllBytes(out))
                    store.delete("jobs/test/binary-$size")
                }
                val client = HttpClient.newHttpClient()
                val signed = store.signedGet("jobs/test/result", "cat_test_40_packed.3mf", 60)
                val response = client.send(HttpRequest.newBuilder(URI(signed)).GET().build(), HttpResponse.BodyHandlers.ofString())
                assertEquals(200, response.statusCode(), response.body())
                assertTrue(response.headers().firstValue("content-disposition").orElse("").contains("cat_test_40_packed.3mf"))
                val unsigned = client.send(HttpRequest.newBuilder(URI("$endpoint/goat/jobs/test/result")).GET().build(), HttpResponse.BodyHandlers.discarding())
                assertEquals(403, unsigned.statusCode())
                store.delete("jobs/test/result")
                store.delete("jobs/test/result")
                assertThrows(Exception::class.java) { store.get("jobs/test/result", dir.resolve("deleted.txt")) }
            }
    }

    private companion object {
        const val S3_PORT = 8333
    }
}
