package com.goatpacker.web.service.server

import com.goatpacker.web.config.PlatformProperties
import com.goatpacker.web.entity.QueueStatus
import com.goatpacker.web.entity.WorkerEntity
import com.goatpacker.web.repository.PlatformRepository
import com.goatpacker.web.repository.QueueJobRepository
import com.goatpacker.web.repository.WorkerRepository
import org.springframework.context.annotation.Profile
import org.springframework.stereotype.Service
import org.springframework.transaction.annotation.Transactional

/**
 * Регистрация и признаки жизни исполнителей. Слот освобождается только после подтверждённой остановки
 * всего, что запускал прошлый процесс исполнителя: иначе два процесса работали бы над одной задачей.
 */
@Service
@Profile("server")
class WorkerNodeService(
    private val workers: WorkerRepository,
    private val jobs: QueueJobRepository,
    private val platform: PlatformRepository,
    private val transitions: JobTransitions,
    private val props: PlatformProperties,
) {
    /** Исполнитель (новая загрузка [bootId]) заявляет о себе с нулевой ёмкостью, пока старые процессы не подтверждены остановленными. */
    @Transactional
    fun beginRecovery(workerId: String, bootId: String, hostId: String = ""): Boolean {
        platform.lockForUpdate()
        val current = workers.findById(workerId).orElse(null)
        check(current == null || current.hostId == "" || current.hostId == hostId) {
            "Worker ID belongs to another Docker host; explicit host fencing is required"
        }
        workers.register(workerId, bootId, props.runnerMode.name.lowercase(), hostId)
        return true
    }

    /** Старые процессы исполнителя остановлены: его задачи освобождают слоты и уходят на повтор. */
    @Transactional
    fun confirmRecovered(workerId: String, bootId: String) {
        platform.lockForUpdate()
        check(workers.countByIdAndBootId(workerId, bootId) == 1L)
        jobs.findAllByWorkerIdAndStatusIn(workerId, QueueStatus.OCCUPYING_SLOT)
            .forEach { transitions.recoverStopped(it, "Исполнитель перезапущен", restarted = true) }
    }

    /** Признак жизни с текущей ёмкостью; false — загрузку исполнителя вытеснила более новая. */
    @Transactional
    fun pulse(node: WorkerEntity): Boolean {
        platform.lockForUpdate()
        return workers.pulse(node.id, node.bootId, node.cpuLimit, node.memoryLimitBytes, node.capacity, node.activeCount) == 1
    }

    /** Штатная остановка: ёмкость обнуляется и исполнитель сразу считается пропавшим. */
    @Transactional
    fun markStopped(workerId: String, bootId: String) {
        platform.lockForUpdate()
        workers.markStopped(workerId, bootId, STOPPED_STALENESS_SECONDS)
    }

    companion object {
        /** Исполнитель, не подававший признаков жизни дольше, не даёт ёмкости. */
        const val FRESH_SECONDS = 30

        /** После остановки время последнего признака жизни отодвигается дальше [FRESH_SECONDS]. */
        private const val STOPPED_STALENESS_SECONDS = 61
    }
}
