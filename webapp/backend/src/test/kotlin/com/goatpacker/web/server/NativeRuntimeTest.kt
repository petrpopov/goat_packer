package com.goatpacker.web.server

import com.fasterxml.jackson.module.kotlin.jacksonObjectMapper
import com.goatpacker.web.config.PlatformProperties
import com.goatpacker.web.entity.newId
import com.goatpacker.web.model.Job
import com.goatpacker.web.model.JobKind
import com.goatpacker.web.service.runner.DockerCli
import com.goatpacker.web.service.runner.NativeLauncher
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertFalse
import org.junit.jupiter.api.Assertions.assertThrows
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Tag
import org.junit.jupiter.api.Test
import java.io.OutputStream
import java.nio.file.Files
import java.nio.file.Path
import java.time.OffsetDateTime
import java.util.concurrent.CompletableFuture
import java.util.concurrent.TimeUnit

/** Explicit real-image qualification: ./gradlew nativeTest. Never silently skipped for missing images. */
@Tag("native")
class NativeRuntimeTest {
    private fun launcher(): NativeLauncher = NativeLauncher(
        PlatformProperties().apply {
            workerId = "native-test-${newId()}"
            workerMemoryMib = 4096
            runnerBambuImage = "goat-runner-bambu:2.9.1.70"
            runnerOrcaImage = "goat-runner-orca:2.4.2"
        },
        jacksonObjectMapper(),
        DockerCli(),
    )

    @Test fun `both real offline slicers export regular files from bounded frozen sandbox`() {
        for (kind in listOf("bambu", "orca")) {
            val launcher = launcher()
            val images = launcher.qualify()
            val root = Files.createTempDirectory("goat-native-selfcontained-")
            val job = Job(newId(), JobKind.PACK, root, OffsetDateTime.now()).apply {
                runtimeImage = images[kind]
                runtimeTimeoutSeconds = 120
                runtimeFence = 1
            }
            try {
                Files.copy(Path.of("../../tests/fixtures/native-p1s.3mf"), root.resolve("input.3mf"))
                Files.writeString(root.resolve("profile.json"), """{"slicer_kind":"$kind"}""")
                val process = launcher.start(job, listOf("slicer", "--slice", "1", "--export-3mf", "packed.3mf", "input.3mf"))
                val log = CompletableFuture.supplyAsync { process.inputStream.readAllBytes().toString(Charsets.UTF_8) }
                assertTrue(process.waitFor(150, TimeUnit.SECONDS), "$kind timeout")
                assertEquals(0, process.exitValue(), "$kind: ${log.get()}")
                launcher.collect(job)
                assertTrue(Files.size(root.resolve("packed.3mf")) > 1000)
                assertFalse(launcher.tracked(job))
            } finally {
                launcher.stop(job)
                root.toFile().deleteRecursively()
            }
        }
    }

    @Test fun `export rejects symlink rather than following it into trusted runtime`() {
        val launcher = launcher()
        val image = launcher.qualify()["bambu"]!!
        val root = Files.createTempDirectory("goat-native-symlink-")
        val job = Job(newId(), JobKind.PACK, root, OffsetDateTime.now()).apply {
            runtimeImage = image
            runtimeFence = 1
        }
        try {
            Files.writeString(root.resolve("input.3mf"), "unused")
            Files.writeString(root.resolve("profile.json"), """{"slicer_kind":"bambu"}""")
            val process = launcher.start(job, listOf("slicer", "--help"))
            process.inputStream.transferTo(OutputStream.nullOutputStream())
            assertEquals(0, process.waitFor())
            val inject = ProcessBuilder("docker", "exec", "goat-${job.id}-1", "ln", "-s", "/etc/passwd", "/work/packed.3mf").start()
            assertEquals(0, inject.waitFor())
            assertThrows(Exception::class.java) { launcher.collect(job) }
            assertFalse(Files.exists(root.resolve("packed.3mf")))
        } finally {
            launcher.stop(job)
            root.toFile().deleteRecursively()
        }
    }
}
