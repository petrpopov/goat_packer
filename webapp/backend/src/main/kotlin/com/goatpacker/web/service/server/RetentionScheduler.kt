package com.goatpacker.web.service.server

import com.goatpacker.web.repository.ObjectGcRepository
import com.goatpacker.web.repository.ObjectStore
import org.springframework.context.annotation.Profile
import org.springframework.scheduling.annotation.Scheduled
import org.springframework.stereotype.Component

/** Раз в минуту: уборка очереди, удаление просроченного из БД и затем — самих объектов хранилища (retry при сбое). */
@Component
@Profile("server")
class RetentionScheduler(
    private val execution: JobExecutionService,
    private val retention: RetentionService,
    private val gc: ObjectGcRepository,
    private val store: ObjectStore,
) {
    @Scheduled(fixedDelay = COLLECT_INTERVAL_MS, initialDelay = COLLECT_INTERVAL_MS)
    fun collect() {
        execution.recoverQueue()
        retention.expire()
        gc.claimDeletions(RetentionPolicy.BATCH_SIZE).forEach { key ->
            // Сбой удаления не страшен: запись остаётся помеченной, и ключ уйдёт при следующей уборке.
            runCatching {
                store.delete(key)
                gc.remove(key)
            }
        }
    }

    private companion object {
        const val COLLECT_INTERVAL_MS = 60_000L
    }
}
