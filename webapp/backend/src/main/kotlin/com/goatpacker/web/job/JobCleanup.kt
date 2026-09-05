package com.goatpacker.web.job

import com.goatpacker.web.config.AppProperties
import org.slf4j.LoggerFactory
import org.springframework.scheduling.annotation.Scheduled
import org.springframework.stereotype.Component
import java.nio.file.Files
import java.time.OffsetDateTime

/**
 * Периодическая уборка: каталоги джобов старше job-ttl-hours удаляются вместе с записями;
 * кроме того, в хранилище держится не более 50 джобов — первыми выкидываются самые старые
 * завершённые.
 */
@Component
class JobCleanup(
    private val store: JobStore,
    private val properties: AppProperties,
) {
    companion object {
        private val log = LoggerFactory.getLogger(JobCleanup::class.java)

        private const val MAX_JOBS = 50
    }

    @Scheduled(fixedDelay = 3_600_000)
    fun cleanup() {
        val now = OffsetDateTime.now()
        val ttlCutoff = now.minusHours(properties.jobTtlHours)

        store.all()
            .filter { (it.current.finishedAt ?: it.createdAt).isBefore(ttlCutoff) }
            .forEach { purge(it, "истёк TTL ${properties.jobTtlHours} ч") }

        // Максимум 50 джобов: сначала самые старые завершённые.
        val running = store.all().filter { it.current.status == JobStatus.RUNNING }
        val finished = store.all()
            .filter { it.current.status == JobStatus.SUCCEEDED || it.current.status == JobStatus.FAILED }
            .sortedBy { it.current.finishedAt ?: it.createdAt }
        if (store.count() > MAX_JOBS) {
            finished.take(store.count() - MAX_JOBS).forEach { purge(it, "превышен лимит джобов") }
        }
        if (running.isNotEmpty()) {
            log.debug("Активных джобов: {}", running.size)
        }
    }

    private fun purge(job: Job, reason: String) {
        log.info("Удаляю джобу {} ({}): {}", job.id, job.kind, reason)
        job.process?.takeIf { it.isAlive }?.let { JobProcessKiller.kill(it) }
        store.remove(job.id)
        deleteDirectory(job.directory)
    }

    private fun deleteDirectory(directory: java.nio.file.Path) {
        try {
            if (Files.exists(directory)) {
                directory.toFile().deleteRecursively()
            }
        } catch (ex: Exception) {
            log.warn("Не удалось удалить каталог джоба {}: {}", directory, ex.message)
        }
    }
}
