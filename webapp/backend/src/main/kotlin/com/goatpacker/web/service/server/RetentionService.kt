package com.goatpacker.web.service.server

import com.goatpacker.web.repository.ObjectGcRepository
import com.goatpacker.web.repository.PlatformRepository
import com.goatpacker.web.repository.QueueJobSqlRepository
import com.goatpacker.web.repository.RetentionRepository
import org.springframework.context.annotation.Profile
import org.springframework.stereotype.Service
import org.springframework.transaction.annotation.Transactional

/**
 * Reference-aware deletion, шаг 1: строки БД с истёкшим сроком удаляются в одной транзакции, а их объекты
 * ставятся в очередь удаления (`object_gc`). Сами объекты уходит удалять [RetentionScheduler] — после коммита.
 */
@Service
@Profile("server")
class RetentionService(
    private val retention: RetentionRepository,
    private val queueJobs: QueueJobSqlRepository,
    private val gc: ObjectGcRepository,
    private val config: PlatformRepository,
    private val probeCache: ProbeCacheService,
) {
    /** Удаляет просроченные задачи, загрузки, блобы, сессии и прочее. */
    @Transactional
    fun expire() {
        config.lockForUpdate()
        // Дети (проверки нарезкой) живут и умирают с родителем: свой срок у них не считается.
        for (id in retention.findExpiredJobIds(RetentionPolicy.BATCH_SIZE)) {
            gc.scheduleArtifactsOfJobTree(id)
            queueJobs.deleteJob(id)
        }
        gc.scheduleExpiredUploadInputs()
        retention.deleteExpiredUploads()
        // Registered diagnostic files expire separately, without expiring their useful 3MF/CSV/XLSX.
        gc.scheduleOldDebugArtifacts(RetentionPolicy.DEBUG_ARTIFACT_DAYS)
        retention.deleteOldDebugArtifacts(RetentionPolicy.DEBUG_ARTIFACT_DAYS)
        for (blob in retention.findUnusedBlobs(RetentionPolicy.BATCH_SIZE)) {
            gc.scheduleDeletion(blob.objectKey, blob.sizeBytes)
            retention.deleteBlob(blob.id)
        }
        probeCache.expire()
        retention.deleteExpiredSessionsResetsAndGrants()
        retention.deleteIdleGuests(RetentionPolicy.IDLE_GUEST_DAYS)
        retention.deleteExpiredChallenges()
        retention.deleteOldComputeBudgets(RetentionPolicy.COMPUTE_BUDGET_HISTORY_DAYS)
        retention.deleteOldRequestLimits(RetentionPolicy.REQUEST_LIMIT_HISTORY_DAYS)
    }
}
