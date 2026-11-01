package com.goatpacker.web.service.local

import com.goatpacker.web.config.AppProperties
import com.goatpacker.web.model.Job
import com.goatpacker.web.model.JobStatus
import com.goatpacker.web.repository.JobStore
import com.goatpacker.web.util.ProcessKiller
import org.slf4j.LoggerFactory
import org.springframework.context.annotation.Profile
import org.springframework.scheduling.annotation.Scheduled
import org.springframework.stereotype.Component
import java.nio.file.Files
import java.nio.file.LinkOption
import java.nio.file.Path
import java.time.OffsetDateTime
import java.util.UUID

/**
 * Периодическая уборка: каталоги джобов старше job-ttl-hours удаляются вместе с записями;
 * кроме того, в хранилище держится не более [MAX_JOBS] джобов — первыми выкидываются самые старые
 * завершённые.
 */
@Component
@Profile("!server")
class JobCleanup(
    private val store: JobStore,
    private val properties: AppProperties,
) {
    private val log = LoggerFactory.getLogger(javaClass)

    @Scheduled(fixedDelay = CLEANUP_INTERVAL_MS)
    fun cleanup() {
        val ttlCutoff = OffsetDateTime.now().minusHours(properties.jobTtlHours)

        store.all()
            .filter { it.current.finishedAt?.isBefore(ttlCutoff) == true }
            .forEach { purge(it, "истёк TTL ${properties.jobTtlHours} ч") }
        evictOldestFinished()
        removeOrphanDirectories(ttlCutoff)

        val running = store.all().count { it.current.status == JobStatus.RUNNING }
        if (running > 0) log.debug("Активных джобов: {}", running)
    }

    /** Максимум [MAX_JOBS] джобов: сначала самые старые завершённые. */
    private fun evictOldestFinished() {
        val excess = store.count() - MAX_JOBS
        if (excess <= 0) return
        store.all()
            .filter { it.current.status == JobStatus.SUCCEEDED || it.current.status == JobStatus.FAILED }
            .sortedBy { it.current.finishedAt ?: it.createdAt }
            .take(excess)
            .forEach { purge(it, "превышен лимит джобов") }
    }

    /** Jobs live in memory; expired orphan directories after a restart also need cleanup. */
    private fun removeOrphanDirectories(ttlCutoff: OffsetDateTime) {
        if (!Files.isDirectory(properties.jobsRootPath)) return
        Files.list(properties.jobsRootPath).use { entries ->
            entries.forEach { dir ->
                val id = dir.fileName.toString()
                val isJobDirectory = runCatching { UUID.fromString(id) }.isSuccess && Files.isDirectory(dir, LinkOption.NOFOLLOW_LINKS)
                if (isJobDirectory && store.get(id) == null && Files.getLastModifiedTime(dir).toInstant().isBefore(ttlCutoff.toInstant())) {
                    deleteDirectory(dir)
                }
            }
        }
    }

    private fun purge(job: Job, reason: String) {
        log.info("Удаляю джобу {} ({}): {}", job.id, job.kind, reason)
        job.cancel()?.takeIf { it.isAlive }?.let(ProcessKiller::kill)
        store.remove(job.id)
        deleteDirectory(job.directory)
    }

    private fun deleteDirectory(directory: Path) {
        try {
            if (Files.exists(directory)) directory.toFile().deleteRecursively()
        } catch (ex: Exception) {
            log.warn("Не удалось удалить каталог джоба {}: {}", directory, ex.message)
        }
    }

    private companion object {
        const val MAX_JOBS = 50
        const val CLEANUP_INTERVAL_MS = 3_600_000L
    }
}
