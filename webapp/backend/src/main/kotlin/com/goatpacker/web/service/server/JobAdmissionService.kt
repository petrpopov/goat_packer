package com.goatpacker.web.service.server

import com.goatpacker.web.config.PlatformProperties
import com.goatpacker.web.entity.PrincipalEntity
import com.goatpacker.web.entity.QueueStatus
import com.goatpacker.web.exception.quotaExceeded
import com.goatpacker.web.repository.QueueJobRepository
import org.springframework.context.annotation.Profile
import org.springframework.stereotype.Service
import java.time.LocalDate

/** Допуск новой задачи в очередь: размер очереди, лимит активных задач владельца, место в хранилище и вычислительный бюджет. */
@Service
@Profile("server")
class JobAdmissionService(
    private val jobs: QueueJobRepository,
    private val quotas: StorageQuotaService,
    private val budget: ComputeBudgetService,
    private val props: PlatformProperties,
) {
    /** Резервирует ресурсы под задачу [owner] и возвращает день, на который списывается бюджет. Нарушение лимита — 429. */
    fun admit(owner: PrincipalEntity): LocalDate {
        if (jobs.countByStatusIn(QueueStatus.ACTIVE) >= props.maxQueue) quotaExceeded("Очередь заполнена — попробуйте позже")
        val ownActive = jobs.countByOwnerIdAndStatusIn(owner.id, QueueStatus.ACTIVE)
        if (props.admissionLimits && ownActive >= props.activeJobsLimit(owner.kind)) quotaExceeded("Достигнут лимит ваших активных задач")
        quotas.reserve(owner, props.outputReservationBytes)
        val day = budget.today()
        budget.reserve(owner, day, props.jobTimeoutSeconds)
        return day
    }
}
