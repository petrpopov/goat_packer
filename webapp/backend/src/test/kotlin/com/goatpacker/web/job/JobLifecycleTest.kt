package com.goatpacker.web.job

import com.fasterxml.jackson.databind.ObjectMapper
import com.goatpacker.web.config.AppProperties
import com.goatpacker.web.dto.ProfileDto
import com.goatpacker.web.dto.ScheduleJobForm
import com.goatpacker.web.exception.BadRequestException
import com.goatpacker.web.service.*
import org.junit.jupiter.api.Assertions.*
import org.junit.jupiter.api.Test
import org.junit.jupiter.api.io.TempDir
import org.junit.jupiter.api.assertThrows
import org.mockito.Mockito
import java.nio.file.Files
import java.nio.file.Path
import java.time.OffsetDateTime
import java.util.UUID
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit

class JobLifecycleTest {
    @TempDir lateinit var root: Path

    private fun properties(bin: Path = root.resolve("not-executed")) = AppProperties(
        packerBin = bin.toString(), schedulerBin = bin.toString(), jobsRoot = root.toString(),
    ).also { it.resolveAbsolutePaths() }

    private fun job(kind: JobKind = JobKind.PACK, created: OffsetDateTime = OffsetDateTime.now()): Job {
        val id = UUID.randomUUID().toString()
        return Job(id, kind, Files.createDirectory(root.resolve(id)), created)
    }

    private fun params() = PackParams("test", root.resolve("profile.json"), 0.7, 0.25, 1, 1, 1, null, "search", "")

    @Test fun `deleted queued job never launches`() {
        val store = JobStore()
        val job = job(); store.put(job)
        JobService(store).delete(job.id)
        JobRunner(properties(), ObjectMapper(), XlsxWriter()).runPack(job, params())
        assertNull(job.process)
        assertFalse(Files.exists(job.directory))
        assertNull(store.get(job.id))
    }

    @Test fun `running cancellation kills worker and removes files`() {
        val bin = root.resolve("slow.sh")
        Files.writeString(bin, "#!/bin/sh\necho started\nsleep 60\n")
        bin.toFile().setExecutable(true)
        val store = JobStore(); val job = job(); store.put(job)
        val executor = Executors.newSingleThreadExecutor()
        try {
            val future = executor.submit { JobRunner(properties(bin), ObjectMapper(), XlsxWriter()).runPack(job, params()) }
            val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(5)
            while (job.process == null && System.nanoTime() < deadline) Thread.sleep(10)
            val process = requireNotNull(job.process)
            JobService(store).delete(job.id)
            future.get(10, TimeUnit.SECONDS)
            assertFalse(process.isAlive)
            assertNull(store.get(job.id))
            assertFalse(Files.exists(job.directory))
        } finally { executor.shutdownNow() }
    }

    @Test fun `TTL preserves old queued and running jobs but purges finished and orphaned ones`() {
        val store = JobStore()
        val old = OffsetDateTime.now().minusDays(2)
        val queued = job(created = old); store.put(queued)
        val running = job(created = old); running.update { it.copy(status = JobStatus.RUNNING) }; store.put(running)
        val finished = job(created = old); finished.update { it.copy(status = JobStatus.SUCCEEDED, finishedAt = old) }; store.put(finished)
        val orphan = root.resolve(UUID.randomUUID().toString()); Files.createDirectory(orphan)
        Files.setLastModifiedTime(orphan, java.nio.file.attribute.FileTime.from(old.toInstant()))
        JobCleanup(store, properties()).cleanup()
        assertNotNull(store.get(queued.id)); assertNotNull(store.get(running.id))
        assertNull(store.get(finished.id)); assertFalse(Files.exists(finished.directory)); assertFalse(Files.exists(orphan))
    }

    @Test fun `schedule owns source even after deleting the pack job`() {
        val store = JobStore(); val pack = job(); store.put(pack)
        pack.packResult = PackJobResult(2, "test", 0.7)
        pack.update { it.copy(status = JobStatus.SUCCEEDED) }
        val bytes = "snapshot source".toByteArray(); Files.write(pack.directory.resolve("packed.3mf"), bytes)
        val profiles = Mockito.mock(ProfileService::class.java)
        Mockito.`when`(profiles.require("test")).thenReturn(ProfileService.ResolvedProfile(
            ProfileDto("test", "Test", "bambu", 256.0, 256.0, 0.0, false), root.resolve("test.json")))
        Mockito.`when`(profiles.require("other")).thenReturn(ProfileService.ResolvedProfile(
            ProfileDto("other", "Other", "orca", 270.0, 270.0, 0.0, false), root.resolve("other.json")))
        val service = ScheduleService(properties(), profiles, store, Mockito.mock(JobRunner::class.java), Project3mf(ObjectMapper()))
        assertThrows<BadRequestException> { service.create(ScheduleJobForm(packJobId = pack.id, profileId = "other")) }
        val scheduled = service.create(ScheduleJobForm(packJobId = pack.id, profileId = "test"))
        JobService(store).delete(pack.id)
        assertArrayEquals(bytes, Files.readAllBytes(store.get(scheduled.jobId)!!.directory.resolve("input.3mf")))
    }

    @Test fun `unfinished artifacts and symlinks are not downloadable`() {
        val store = JobStore(); val job = job(); store.put(job)
        Files.writeString(job.directory.resolve("packed.3mf"), "partial")
        val service = JobService(store)
        assertThrows<com.goatpacker.web.exception.ResourceNotFoundException> { service.artifact(job.id, "packed.3mf") }
        job.update { it.copy(status = JobStatus.SUCCEEDED) }
        Files.delete(job.directory.resolve("packed.3mf"))
        val outside = root.resolve("outside"); Files.writeString(outside, "secret")
        Files.createSymbolicLink(job.directory.resolve("packed.3mf"), outside)
        assertThrows<com.goatpacker.web.exception.ResourceNotFoundException> { service.artifact(job.id, "packed.3mf") }
    }
}
