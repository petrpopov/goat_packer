package com.goatpacker.web.service.server

import com.goatpacker.web.config.PlatformProperties
import com.goatpacker.web.entity.PrincipalEntity
import com.goatpacker.web.entity.QueueJobEntity
import com.goatpacker.web.exception.quotaExceeded
import com.goatpacker.web.repository.ComputeBudgetRepository
import com.goatpacker.web.repository.ComputeBudgetRepository.Companion.GLOBAL_OWNER
import org.springframework.context.annotation.Profile
import org.springframework.stereotype.Service
import java.time.Duration
import java.time.Instant
import java.time.LocalDate
import java.time.ZoneOffset

/**
 * Суточный вычислительный бюджет: при постановке задачи резервируется её лимит времени, при остановке
 * фактически потраченное переходит в «использовано», остаток возвращается. Учёт ведётся на владельца и
 * на сервис целиком ([GLOBAL_OWNER]).
 */
@Service
@Profile("server")
class ComputeBudgetService(
    private val budgets: ComputeBudgetRepository,
    private val props: PlatformProperties,
) {
    /** День бюджета — по UTC, чтобы сутки не зависели от пояса сервера. */
    fun today(): LocalDate = LocalDate.now(ZoneOffset.UTC)

    /** Резервирует [seconds] на день [day]; без лимитов (`admissionLimits=false`) резерв пишется, но не отказывает: учёт остаётся. */
    fun reserve(owner: PrincipalEntity, day: LocalDate, seconds: Long) {
        val accounts = listOf(owner.id to props.computeBudgetSeconds(owner.kind), GLOBAL_OWNER to props.globalComputeSeconds)
        for ((account, limit) in accounts) {
            val committed = budgets.committedSeconds(account, day)
            if (props.admissionLimits && seconds > limit - committed) quotaExceeded("Суточный вычислительный бюджет исчерпан")
            budgets.reserve(account, day, seconds)
        }
    }

    /** Списывает время текущей попытки из резерва в использованное и закрывает попытку (`startedAt = null`). */
    fun chargeAttempt(job: QueueJobEntity) {
        val startedAt = job.startedAt
        val used = if (startedAt != null && job.computeReservedSeconds > 0) {
            Duration.between(startedAt, Instant.now()).seconds.coerceIn(1, job.computeReservedSeconds)
        } else {
            0L
        }
        val day = job.budgetDay
        if (used > 0 && day != null) {
            for (account in accountsOf(job)) budgets.charge(account, day, used)
        }
        job.computeReservedSeconds -= used
        job.startedAt = null
    }

    /** Задача завершена: остаток резерва возвращается в бюджет. */
    fun releaseRemaining(job: QueueJobEntity) {
        val day = job.budgetDay
        if (job.computeReservedSeconds > 0 && day != null) {
            for (account in accountsOf(job)) budgets.release(account, day, job.computeReservedSeconds)
        }
        job.computeReservedSeconds = 0
    }

    private fun accountsOf(job: QueueJobEntity) = listOf(job.ownerId, GLOBAL_OWNER)
}
