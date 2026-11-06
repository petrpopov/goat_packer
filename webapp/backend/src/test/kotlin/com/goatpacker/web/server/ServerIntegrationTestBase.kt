package com.goatpacker.web.server

import com.fasterxml.jackson.databind.JsonNode
import com.fasterxml.jackson.databind.ObjectMapper
import com.goatpacker.web.FakeBins
import com.goatpacker.web.config.AppProperties
import com.goatpacker.web.config.PlatformProperties
import com.goatpacker.web.descriptorProject3mf
import com.goatpacker.web.entity.QueueStatus
import com.goatpacker.web.entity.newId
import com.goatpacker.web.repository.BlobRepository
import com.goatpacker.web.repository.ObjectGcRepository
import com.goatpacker.web.repository.ObjectStore
import com.goatpacker.web.repository.QueueJobRepository
import com.goatpacker.web.repository.WorkerRepository
import com.goatpacker.web.service.runner.JobRunner
import com.goatpacker.web.service.runner.NativeLauncher
import com.goatpacker.web.service.server.JobExecutionService
import com.goatpacker.web.service.server.ProbeCacheService
import com.goatpacker.web.service.server.QueueWorker
import com.goatpacker.web.service.server.WorkerJobProcessor
import com.goatpacker.web.service.server.WorkerNodeService
import jakarta.servlet.http.Cookie
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.BeforeEach
import org.springframework.beans.factory.annotation.Autowired
import org.springframework.boot.test.autoconfigure.web.servlet.AutoConfigureMockMvc
import org.springframework.boot.test.context.SpringBootTest
import org.springframework.http.HttpMethod
import org.springframework.jdbc.core.JdbcTemplate
import org.springframework.mock.web.MockHttpServletResponse
import org.springframework.test.context.ActiveProfiles
import org.springframework.test.context.DynamicPropertyRegistry
import org.springframework.test.context.DynamicPropertySource
import org.springframework.test.web.servlet.MockMvc
import org.springframework.test.web.servlet.request.MockMvcRequestBuilders.request
import org.testcontainers.containers.PostgreSQLContainer
import java.nio.file.Files
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger
import org.junit.jupiter.api.Tag

/**
 * Серверный режим целиком: настоящий PostgreSQL в контейнере, хранилище в каталоге, фейковые бинарники и
 * локальный раннер. Все наследники делят один контекст Spring и одну базу.
 */
/** Testcontainers: off in `gradle test`, run by `containersTest`. */
@Tag("containers")
@SpringBootTest
@AutoConfigureMockMvc
@ActiveProfiles("server")
abstract class ServerIntegrationTestBase {
    @Autowired protected lateinit var mvc: MockMvc
    @Autowired protected lateinit var json: ObjectMapper
    @Autowired protected lateinit var jdbc: JdbcTemplate
    @Autowired protected lateinit var execution: JobExecutionService
    @Autowired protected lateinit var nodeService: WorkerNodeService
    @Autowired protected lateinit var gc: ObjectGcRepository
    @Autowired protected lateinit var jobs: QueueJobRepository
    @Autowired protected lateinit var blobs: BlobRepository
    @Autowired protected lateinit var nodes: WorkerRepository
    @Autowired protected lateinit var store: ObjectStore
    @Autowired protected lateinit var runner: JobRunner
    @Autowired protected lateinit var launcher: NativeLauncher
    @Autowired protected lateinit var app: AppProperties
    @Autowired protected lateinit var props: PlatformProperties
    @Autowired protected lateinit var probeCache: ProbeCacheService

    companion object {
        class Pg : PostgreSQLContainer<Pg>("postgres:17.0")

        private val postgres = Pg().apply { start() }
        private val root = Files.createTempDirectory("goat-server-test-")
        private val clientIds = AtomicInteger(0)

        const val ORIGIN = "http://localhost:3007"
        const val ADMIN_LOGIN = "admin"
        const val ADMIN_PASSWORD = "Test-admin-secret-123"
        private const val AWAIT_JOB_SECONDS = 15L
        private const val AWAIT_POLL_MS = 50L

        @JvmStatic
        @DynamicPropertySource
        fun properties(registry: DynamicPropertyRegistry) {
            registry.add("spring.datasource.url", postgres::getJdbcUrl)
            registry.add("spring.datasource.username", postgres::getUsername)
            registry.add("spring.datasource.password", postgres::getPassword)
            registry.add("platform.storage-mode") { "file" }
            registry.add("platform.local-storage-root") { root.resolve("objects").toString() }
            registry.add("platform.admin-login") { ADMIN_LOGIN }
            registry.add("platform.admin-password") { ADMIN_PASSWORD }
            registry.add("platform.worker-enabled") { false }
            registry.add("platform.secure-cookies") { false }
            registry.add("platform.runner-mode") { "local" }
            registry.add("platform.allow-local-runner") { true }
            registry.add("app.packer-bin") { FakeBins.install(root, "fake-packer.sh") }
            registry.add("app.scheduler-bin") { FakeBins.install(root, "fake-scheduler.sh") }
            registry.add("app.calibration-bin") { FakeBins.install(root, "fake-calibration.sh") }
            registry.add("app.jobs-root") { root.resolve("work").toString() }
        }

        /** Каталог, общий для всех тестов: туда складываются фейковые бинарники и пресеты. */
        fun sharedRoot() = root
    }

    /** HTTP-клиент одного пользователя: свои cookie и адрес (лимиты частоты считаются по адресу). */
    protected inner class Client {
        private val address = "10.10.0.${clientIds.incrementAndGet()}"
        private val cookies = linkedMapOf<String, Cookie>()

        init {
            call("GET", "/api/v2/auth/csrf")
        }

        fun call(
            method: String,
            path: String,
            body: Any? = null,
            expected: Int = 200,
            headers: Map<String, String> = emptyMap(),
        ): JsonNode {
            val builder = request(HttpMethod.valueOf(method), path).with {
                it.remoteAddr = address
                it
            }
            if (cookies.isNotEmpty()) builder.cookie(*cookies.values.toTypedArray())
            if (method !in listOf("GET", "HEAD")) cookies["XSRF-TOKEN"]?.let { builder.header("X-XSRF-TOKEN", it.value) }
            headers.forEach { (key, value) -> builder.header(key, value) }
            if (body is ByteArray) {
                builder.contentType("application/octet-stream").content(body)
            } else if (body != null) {
                builder.contentType("application/json").content(json.writeValueAsBytes(body))
            }
            val response = mvc.perform(builder).andReturn().response
            rememberCookies(response)
            assertEquals(expected, response.status, "$method $path: ${response.contentAsString}")
            return if (response.contentAsString.isBlank()) json.createObjectNode() else json.readTree(response.contentAsString)
        }

        fun bytes(path: String, expected: Int = 200): ByteArray {
            val builder = request(HttpMethod.GET, path).with {
                it.remoteAddr = address
                it
            }
            if (cookies.isNotEmpty()) builder.cookie(*cookies.values.toTypedArray())
            val response = mvc.perform(builder).andReturn().response
            assertEquals(expected, response.status, "GET $path")
            return response.contentAsByteArray
        }

        /** Ответ целиком (заголовки, кодировка) — cookie из него запоминаются, статус не проверяется. */
        fun raw(method: String, path: String): MockHttpServletResponse {
            val builder = request(HttpMethod.valueOf(method), path).with {
                it.remoteAddr = address
                it
            }
            if (cookies.isNotEmpty()) builder.cookie(*cookies.values.toTypedArray())
            if (method !in listOf("GET", "HEAD")) cookies["XSRF-TOKEN"]?.let { builder.header("X-XSRF-TOKEN", it.value) }
            val response = mvc.perform(builder).andReturn().response
            rememberCookies(response)
            return response
        }

        fun guest(): JsonNode = call("POST", "/api/v2/auth/guest")

        fun login(name: String = ADMIN_LOGIN, pass: String = ADMIN_PASSWORD, expected: Int = 200): JsonNode =
            call("POST", "/api/v2/auth/login", mapOf("login" to name, "password" to pass), expected)

        /** Загружает проект принтера [printer] и проверяет, что он распознан как [profile]; возвращает id файла. */
        fun upload(
            printer: String = "Test Corner Bed",
            profile: String = "test_corner_bed",
            data: ByteArray = descriptorProject3mf(printer),
        ): String {
            val id = call("POST", "/api/v2/uploads", mapOf("name" to "cat.3mf", "sizeBytes" to data.size))["fileId"].asText()
            val uploaded = call("PUT", "/api/v2/uploads/$id/content", data, headers = mapOf("Origin" to ORIGIN))
            assertEquals(profile, uploaded["profileId"].asText())
            return id
        }

        fun uploadRaw(name: String, data: ByteArray): JsonNode {
            val id = call("POST", "/api/v2/uploads", mapOf("name" to name, "sizeBytes" to data.size))["fileId"].asText()
            return call("PUT", "/api/v2/uploads/$id/content", data, headers = mapOf("Origin" to ORIGIN))
        }

        fun job(
            file: String,
            key: String = newId(),
            extra: Map<String, Any> = emptyMap(),
            expected: Int = 200,
        ): JsonNode = call(
            "POST",
            "/api/v2/jobs",
            mapOf("fileId" to file, "profileId" to "test_corner_bed") + extra,
            expected,
            headers = mapOf("Idempotency-Key" to key),
        )

        /** `POST /api/v2/jobs` с произвольным телом и новым ключом идемпотентности. */
        fun send(body: Map<String, Any?>, expected: Int): JsonNode =
            call("POST", "/api/v2/jobs", body, expected, headers = mapOf("Idempotency-Key" to newId()))

        private fun rememberCookies(response: MockHttpServletResponse) {
            response.getHeaders("Set-Cookie").forEach { header ->
                val pair = header.substringBefore(';').split('=', limit = 2)
                cookies[pair[0]] = Cookie(pair[0], pair[1])
            }
        }
    }

    /** Уникальное число для имён пользователей: база общая для всех тестов. */
    protected fun nextNumber(): Int = clientIds.incrementAndGet()

    @BeforeEach
    fun clearQueue() {
        for (table in listOf("compute_budgets", "bot_challenges", "object_gc", "probe_cache_entries", "worker_nodes", "artifacts", "queue_jobs", "uploads", "blobs")) {
            jdbc.update("delete from $table")
        }
        jdbc.update("update platform_config set max_parallel=1,paused=false where id=1")
    }

    /** Исполнитель собирается вручную: бин создаётся только при worker-enabled, а тесты вызывают dispatch() сами. */
    protected fun newWorker() = QueueWorker(
        execution,
        nodeService,
        WorkerJobProcessor(execution, blobs, gc, store, runner, launcher, app, props, json, probeCache),
        launcher,
        app,
        props,
    )

    /** Ждёт, пока задача перестанет быть активной (успех, ошибка, отмена). */
    protected fun awaitSettled(id: String) {
        val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(AWAIT_JOB_SECONDS)
        while (jobs.findById(id).orElseThrow().status in QueueStatus.ACTIVE && System.nanoTime() < deadline) {
            Thread.sleep(AWAIT_POLL_MS)
        }
    }

    /** Исполнитель на время [block]; по выходу останавливается и освобождает блокировку каталога. */
    protected fun withWorker(block: (QueueWorker) -> Unit) {
        val worker = newWorker()
        try {
            block(worker)
        } finally {
            worker.stop()
        }
    }

    /** Один проход исполнителя и ожидание, пока задача [id] не завершится. */
    protected fun QueueWorker.runUntilSettled(id: String) {
        dispatch()
        awaitSettled(id)
    }
}
