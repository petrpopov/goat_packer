package com.goatpacker.web.service.server

import com.goatpacker.web.config.PlatformProperties
import com.goatpacker.web.entity.ArtifactEntity
import com.goatpacker.web.entity.QueueJobEntity
import com.goatpacker.web.entity.QueueStatus
import com.goatpacker.web.entity.newId
import com.goatpacker.web.repository.ArtifactRepository
import com.goatpacker.web.repository.JobAttemptRepository
import com.goatpacker.web.repository.ObjectGcRepository
import com.goatpacker.web.repository.OperationalStateRepository
import com.goatpacker.web.repository.PlatformRepository
import com.goatpacker.web.repository.PrincipalRepository
import com.goatpacker.web.repository.QueueJobRepository
import com.goatpacker.web.repository.QueueJobSqlRepository
import com.goatpacker.web.repository.WorkerRepository
import org.springframework.context.annotation.Profile
import org.springframework.stereotype.Service
import org.springframework.transaction.annotation.Transactional
import java.time.Instant

/**
 * Протокол исполнителя с очередью: взять задачу, сообщать прогресс, опубликовать результат или освободить слот.
 * Все изменения защищены «забором» ([QueueJobEntity.fence]): запоздавший исполнитель старой попытки ничего не запишет.
 */
@Service
@Profile("server")
class JobExecutionService(
    private val jobs: QueueJobRepository,
    private val owners: PrincipalRepository,
    private val artifacts: ArtifactRepository,
    private val workers: WorkerRepository,
    private val platform: PlatformRepository,
    private val attempts: JobAttemptRepository,
    private val gc: ObjectGcRepository,
    private val queueSql: QueueJobSqlRepository,
    private val operationalState: OperationalStateRepository,
    private val budget: ComputeBudgetService,
    private val transitions: JobTransitions,
    private val props: PlatformProperties,
) {
    /** Отдаёт исполнителю самую старую подходящую задачу или null (пауза, обслуживание, нет слотов, нет задач). */
    @Transactional
    fun claim(workerId: String, bootId: String? = null): QueueJobEntity? {
        val config = platform.lockForUpdate()
        if (config.paused || config.maxParallel == 0 || operationalState.isMaintenance()) return null
        if (bootId != null && workers.countByIdAndBootId(workerId, bootId) == 0L) return null
        val freshSeconds = WorkerNodeService.FRESH_SECONDS
        val capacity = workers.sumFreshCapacity(freshSeconds)
        val ownCapacity = workers.findFreshCapacity(workerId, freshSeconds) ?: return null
        if (jobs.countByWorkerIdAndStatusIn(workerId, QueueStatus.OCCUPYING_SLOT) >= ownCapacity) return null
        if (jobs.countByStatusIn(QueueStatus.OCCUPYING_SLOT) >= minOf(config.maxParallel, capacity)) return null

        val id = queueSql.lockNextClaimableJobId() ?: return null
        val job = jobs.findById(id).orElseThrow()
        if (job.budgetDay == null) reserveBudgetForLegacyJob(job) // jobs accepted before the V2 migration
        val now = Instant.now()
        job.status = QueueStatus.RUNNING
        job.workerId = workerId
        job.fence++
        job.startedAt = now
        job.leaseUntil = now.plusSeconds(LEASE_SECONDS)
        val duration = if (job.computeReservedSeconds > 0) minOf(props.jobTimeoutSeconds, job.computeReservedSeconds) else props.jobTimeoutSeconds
        job.deadlineAt = now.plusSeconds(duration)
        job.attemptCount++
        attempts.start(newId(), job.id, job.fence, workerId, bootId ?: UNSPECIFIED_BOOT_ID)
        return job
    }

    /** Закрепляет образ раннера за задачей на первую попытку; повторные попытки идут на тот же образ. */
    @Transactional
    fun pinImage(id: String, fence: Long, image: String): String {
        val job = jobs.findByIdForUpdate(id) ?: error("Job missing")
        check(job.fence == fence && job.status == QueueStatus.RUNNING)
        if (job.runnerImage == null) job.runnerImage = image
        return job.runnerImage!!
    }

    /** Записывает прогресс; false — попытка устарела, исполнитель должен остановиться. */
    @Transactional
    fun reportProgress(
        id: String,
        fence: Long,
        phase: String,
        percent: Int,
        message: String?,
        stageJson: String? = null,
        layoutJson: String? = null,
    ): Boolean {
        val job = jobs.findByIdForUpdate(id) ?: return false
        if (job.fence != fence || job.status !in QueueStatus.EXECUTING) return false
        job.phase = phase
        job.progress = percent.coerceIn(0, MAX_RUNNING_PROGRESS)
        job.message = message?.take(MAX_MESSAGE_LENGTH)
        job.leaseUntil = Instant.now().plusSeconds(LEASE_SECONDS)
        job.stageJson = stageJson?.take(MAX_STAGE_JSON_LENGTH)
        // Раскладку не обрезаем (обрезанный JSON не разобрать; пакер целится в ≤ 16 КБ, но при сотнях объектов строка может быть длиннее — такую трекер уже отбросил как неразбираемую); null — прежняя остаётся.
        if (layoutJson != null) job.layoutJson = layoutJson
        return true
    }

    /** Продлевает аренду; false — задачу надо остановить (отмена, дедлайн или новая попытка). */
    @Transactional
    fun extendLease(id: String, fence: Long): Boolean {
        val job = jobs.findByIdForUpdate(id) ?: return false
        val stale = job.fence != fence || job.status !in QueueStatus.EXECUTING
        if (stale || job.cancelRequested || job.deadlineAt?.isBefore(Instant.now()) == true) return false
        job.leaseUntil = Instant.now().plusSeconds(LEASE_SECONDS)
        return true
    }

    /** Публикует результат удачной попытки. false — попытка устарела или задача отменена/просрочена/упала. */
    @Transactional
    fun complete(id: String, fence: Long, resultJson: String?, outputs: List<ArtifactEntity>, error: String?): Boolean {
        platform.lockForUpdate()
        val job = jobs.findByIdForUpdate(id) ?: return false
        if (job.fence != fence || job.status != QueueStatus.RUNNING) return false
        if (job.cancelRequested) return cancelled(job)
        if (job.deadlineAt?.isBefore(Instant.now()) == true) return failed(job, "Время выполнения превышено")
        if (error != null) return failed(job, error)
        if (outputs.sumOf { it.sizeBytes } > job.reservedBytes) return failed(job, "Размер результата превышает зарезервированную квоту")
        publish(outputs)
        job.resultJson = resultJson
        job.progress = FINISHED_PROGRESS
        transitions.terminate(job, QueueStatus.SUCCEEDED, null)
        return true
    }

    /** Попытка упала, но оставила диагностику (журнал): публикует её и закрывает задачу как FAILED. */
    @Transactional
    fun completeFailed(id: String, fence: Long, outputs: List<ArtifactEntity>, error: String): Boolean {
        platform.lockForUpdate()
        val job = jobs.findByIdForUpdate(id) ?: return false
        if (job.fence != fence || job.status != QueueStatus.RUNNING) return false
        if (job.cancelRequested) return cancelled(job)
        if (outputs.sumOf { it.sizeBytes } > job.reservedBytes) return false
        publish(outputs)
        transitions.terminate(job, QueueStatus.FAILED, error)
        return true
    }

    /** Уборка очереди: снимает отменённые и протухшие в ожидании задачи, помечает потерянными задачи с просроченной арендой. */
    @Transactional
    fun recoverQueue() {
        platform.lockForUpdate()
        val now = Instant.now()
        val waiting = QueueStatus.WAITING.flatMap { jobs.findAllByStatusOrderByCreatedAtAscIdAsc(it) }
        for (job in waiting) {
            val ownerBlockedOrGone = owners.findById(job.ownerId).orElse(null)?.blocked != false
            if (job.cancelRequested || ownerBlockedOrGone) {
                transitions.terminate(job, QueueStatus.CANCELLED, null)
            } else if (job.createdAt.isBefore(now.minusSeconds(RetentionPolicy.QUEUE_WAIT_LIMIT_SECONDS))) {
                transitions.terminate(job, QueueStatus.EXPIRED, "Превышено время ожидания в очереди")
            }
        }
        // LOST retains its slot until the controller confirms termination; only then may it retry.
        jobs.findAllByStatusOrderByCreatedAtAscIdAsc(QueueStatus.RUNNING)
            .filter { it.leaseUntil?.isBefore(now) == true }
            .forEach {
                it.status = QueueStatus.LOST
                it.error = "Исполнитель потерял связь; слот заблокирован до подтверждения остановки"
            }
    }

    /** Исполнитель закончил, не опубликовав результат: слот освобождается, задача идёт на повтор, если есть попытки. */
    @Transactional
    fun releaseStopped(id: String, fence: Long) = retryStopped(id, fence, "Исполнитель остановлен до завершения задачи")

    /** Не удалось подтвердить остановку процесса: слот остаётся занятым (LOST), пока уборка не подтвердит остановку. */
    @Transactional
    fun markRuntimeLost(id: String, fence: Long, reason: String) {
        platform.lockForUpdate()
        val job = jobs.findByIdForUpdate(id) ?: return
        if (job.fence == fence && job.status in QueueStatus.EXECUTING) {
            job.status = QueueStatus.LOST
            job.error = reason.take(JobTransitions.MAX_ERROR_LENGTH)
        }
    }

    /** Остановка подтверждена: задача получает новую попытку или завершается, если попытки кончились. */
    @Transactional
    fun retryStopped(id: String, fence: Long, reason: String) {
        platform.lockForUpdate()
        val job = jobs.findByIdForUpdate(id) ?: return
        if (job.fence == fence && job.status in QueueStatus.OCCUPYING_SLOT) transitions.recoverStopped(job, reason)
    }

    private fun reserveBudgetForLegacyJob(job: QueueJobEntity) {
        val owner = owners.findById(job.ownerId).orElseThrow()
        val day = budget.today()
        budget.reserve(owner, day, props.jobTimeoutSeconds)
        job.budgetDay = day
        job.computeReservedSeconds = props.jobTimeoutSeconds
    }

    private fun cancelled(job: QueueJobEntity): Boolean {
        transitions.terminate(job, QueueStatus.CANCELLED, null)
        return false
    }

    private fun failed(job: QueueJobEntity, error: String): Boolean {
        transitions.terminate(job, QueueStatus.FAILED, error)
        return false
    }

    /** Фиксирует артефакты: резерв под каждый объект должен быть жив, после публикации GC-запись снимается. */
    private fun publish(outputs: List<ArtifactEntity>) {
        for (output in outputs) check(gc.lockLiveStaging(output.objectKey)) { "Artifact staging expired" }
        artifacts.saveAll(outputs)
        outputs.forEach { gc.remove(it.objectKey) }
    }

    private companion object {
        const val LEASE_SECONDS = 60L
        const val MAX_RUNNING_PROGRESS = 99
        const val FINISHED_PROGRESS = 100
        const val MAX_MESSAGE_LENGTH = 1000
        const val MAX_STAGE_JSON_LENGTH = 4000
        const val UNSPECIFIED_BOOT_ID = "local-test"
    }
}
