package com.goatpacker.web.service.server

import com.goatpacker.web.entity.QueueStatus
import com.goatpacker.web.repository.ObjectGcRepository
import com.goatpacker.web.repository.PlatformRepository
import com.goatpacker.web.repository.QueueJobSqlRepository
import org.springframework.context.annotation.Profile
import org.springframework.stereotype.Service
import org.springframework.transaction.annotation.Transactional

/** Отмена и удаление задач. */
@Service
@Profile("server")
class JobCancellationService(
    private val access: JobAccessService,
    private val transitions: JobTransitions,
    private val platform: PlatformRepository,
    private val gc: ObjectGcRepository,
    private val queueSql: QueueJobSqlRepository,
) {
    @Transactional
    fun cancel(id: String, ownerId: String) {
        platform.lockForUpdate()
        transitions.requestCancel(access.findOwned(id, ownerId))
    }

    /** Удаление: активную задачу сначала останавливают (повторный вызов удалит), завершённую убирают с её артефактами. */
    @Transactional
    fun delete(id: String, ownerId: String) {
        platform.lockForUpdate()
        val job = access.findOwned(id, ownerId)
        if (job.status in QueueStatus.ACTIVE) {
            transitions.requestCancel(job)
            return
        }
        gc.scheduleArtifactsOfJob(id)
        queueSql.deleteJob(id)
    }
}
