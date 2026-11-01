package com.goatpacker.web.service.server

import com.goatpacker.web.config.PlatformProperties
import com.goatpacker.web.exception.ErrorMessages
import com.goatpacker.web.exception.quotaExceeded
import com.goatpacker.web.repository.RequestLimitRepository
import com.goatpacker.web.util.SECONDS_PER_MINUTE
import org.springframework.context.annotation.Profile
import org.springframework.stereotype.Service
import org.springframework.transaction.annotation.Transactional

/** Не более [maximum] обращений за окно в [windowSeconds] секунд. */
data class RateRule(val maximum: Int, val windowSeconds: Long = SECONDS_PER_MINUTE)

/** Ограничитель частоты: счётчики окон в БД, поэтому лимиты общие для всех экземпляров API. */
@Service
@Profile("server")
class RateLimiter(
    private val limits: RequestLimitRepository,
    private val props: PlatformProperties,
) {
    /** Засчитывает обращение по [key]; при превышении — 429 с [message]. Без лимитов (`admissionLimits=false`) не считает. */
    @Transactional
    fun check(key: String, rule: RateRule, message: String = ErrorMessages.TOO_MANY_REQUESTS) {
        if (!props.admissionLimits) return
        if (limits.recordHit(key, rule.windowSeconds) > rule.maximum) quotaExceeded(message)
    }
}
