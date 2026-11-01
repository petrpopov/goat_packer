package com.goatpacker.web.service.server

import com.goatpacker.web.config.PlatformProperties
import com.goatpacker.web.entity.PrincipalKind
import com.goatpacker.web.entity.QueueJobEntity
import com.goatpacker.web.entity.QueueStatus
import com.goatpacker.web.repository.JobAttemptRepository
import com.goatpacker.web.repository.PrincipalRepository
import com.goatpacker.web.repository.QueueJobSqlRepository
import org.springframework.context.annotation.Profile
import org.springframework.stereotype.Service
import java.time.Instant

/**
 * Переходы задачи между состояниями вместе с побочными эффектами: возврат бюджета, закрытие попытки, срок
 * хранения. Вызывается только внутри транзакций вызывающих сервисов (под глобальной блокировкой платформы).
 */
@Service
@Profile("server")
class JobTransitions(
    private val owners: PrincipalRepository,
    private val attempts: JobAttemptRepository,
    private val budget: ComputeBudgetService,
    private val queueSql: QueueJobSqlRepository,
    private val props: PlatformProperties,
) {
    /** Просит остановить активную задачу: ожидающая снимается сразу, идущая — когда исполнитель увидит флаг. */
    fun requestCancel(job: QueueJobEntity) {
        if (job.status !in QueueStatus.ACTIVE) return
        job.cancelRequested = true
        if (job.status in QueueStatus.WAITING) terminate(job, QueueStatus.CANCELLED, null)
    }

    /** Задача завершена окончательно ([status] — SUCCEEDED, FAILED, CANCELLED или EXPIRED). */
    fun terminate(job: QueueJobEntity, status: QueueStatus, error: String?) {
        val startedAt = job.startedAt
        budget.chargeAttempt(job)
        job.startedAt = startedAt
        budget.releaseRemaining(job)
        attempts.finish(job.id, job.fence, status, error?.take(MAX_ERROR_LENGTH))
        job.status = status
        job.error = error?.take(MAX_ERROR_LENGTH)
        job.finishedAt = Instant.now()
        job.leaseUntil = null
        job.reservedBytes = 0
        val isGuest = owners.findById(job.ownerId).orElseThrow().kind == PrincipalKind.GUEST
        job.expiresAt = if (isGuest) Instant.now().plusSeconds(RetentionPolicy.GUEST_CONTENT_TTL_SECONDS) else null
        // Результат проверки живёт внутри родителя: срок хранения гостя считается и от её конца, иначе её файлы ушли бы раньше, чем появились.
        val parentId = job.parentJobId
        val expiresAt = job.expiresAt
        if (parentId != null && expiresAt != null) queueSql.extendExpiry(parentId, expiresAt)
    }

    /**
     * Исполнитель остановлен или потерян: задача уходит на повторный запуск, пока есть попытки и бюджет.
     * [restarted]: исполнитель поднялся заново (так бывает при каждом обновлении сервера) — пользователю важно,
     * почему задача пошла сначала.
     */
    fun recoverStopped(job: QueueJobEntity, reason: String, restarted: Boolean = false) {
        budget.chargeAttempt(job)
        attempts.finish(job.id, job.fence, QueueStatus.LOST, reason.take(MAX_ERROR_LENGTH))
        if (job.cancelRequested || owners.findById(job.ownerId).orElseThrow().blocked) {
            terminate(job, QueueStatus.CANCELLED, null)
            return
        }
        if (job.attemptCount >= props.maxRetries + 1 || job.computeReservedSeconds <= 0) {
            terminate(job, QueueStatus.FAILED, reason)
            return
        }
        job.status = QueueStatus.RETRY_WAIT
        job.message = if (restarted) "Сервер перезапускался — задача запущена заново" else "Сбой исполнителя — задача запущена заново"
        job.stageJson = null
        job.layoutJson = null
        job.error = null
        job.workerId = null
        job.leaseUntil = null
        job.startedAt = null
        job.deadlineAt = null
        job.progress = 0
        job.availableAt = Instant.now().plusSeconds(RETRY_DELAY_SECONDS)
    }

    companion object {
        /** Длина колонки `queue_jobs.error` и `job_attempts.message`. */
        const val MAX_ERROR_LENGTH = 2000

        private const val RETRY_DELAY_SECONDS = 10L
    }
}
