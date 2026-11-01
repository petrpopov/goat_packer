package com.goatpacker.web.service.server

import com.goatpacker.web.entity.QueueStatus
import com.goatpacker.web.exception.notFound
import com.goatpacker.web.repository.ObjectGcRepository
import com.goatpacker.web.repository.PlatformRepository
import com.goatpacker.web.repository.QueueJobRepository
import com.goatpacker.web.repository.QueueJobSqlRepository
import org.springframework.context.annotation.Profile
import org.springframework.stereotype.Service
import org.springframework.transaction.annotation.Transactional

/** Отмена и удаление задач вместе с их проверками нарезкой. */
@Service
@Profile("server")
class JobCancellationService(
    private val jobs: QueueJobRepository,
    private val access: JobAccessService,
    private val transitions: JobTransitions,
    private val platform: PlatformRepository,
    private val gc: ObjectGcRepository,
    private val queueSql: QueueJobSqlRepository,
) {
    /** Отмена идущей проверки нарезкой; упаковка остаётся как была. */
    @Transactional
    fun cancelCheck(parentId: String, ownerId: String) {
        val parent = access.findOwned(parentId, ownerId)
        parent.checkJobId?.let { cancel(it, ownerId) }
    }

    @Transactional
    fun cancel(id: String, ownerId: String) {
        platform.lockForUpdate()
        transitions.requestCancel(access.findOwned(id, ownerId))
    }

    /** Удаление: активную задачу сначала останавливают (повторный вызов удалит), завершённую убирают вместе с проверками. */
    @Transactional
    fun delete(id: String, ownerId: String) {
        platform.lockForUpdate()
        val job = access.findOwned(id, ownerId)
        if (job.parentJobId != null) notFound()
        if (job.status in QueueStatus.ACTIVE) {
            transitions.requestCancel(job)
            return
        }
        // Идущая проверка нарезкой: каскад удалил бы строку из-под работающего воркера — сначала отмена, удаление повторяется.
        val running = jobs.findAllByParentJobId(id).filter { it.status in QueueStatus.ACTIVE }
        if (running.isNotEmpty()) {
            running.forEach(transitions::requestCancel)
            if (running.any { it.status in QueueStatus.ACTIVE }) return
            jobs.flush()
        }
        // Артефакты проверок (дети удаляются каскадом) уходят в GC вместе с родительскими.
        gc.scheduleArtifactsOfJobTree(id)
        queueSql.deleteJob(id)
    }
}
