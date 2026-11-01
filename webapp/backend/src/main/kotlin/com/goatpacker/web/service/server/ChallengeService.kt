package com.goatpacker.web.service.server

import com.goatpacker.web.config.PlatformProperties
import com.goatpacker.web.dto.ChallengeDto
import com.goatpacker.web.entity.newId
import com.goatpacker.web.exception.badRequest
import com.goatpacker.web.repository.ChallengeRepository
import com.goatpacker.web.security.Secrets
import org.springframework.context.annotation.Profile
import org.springframework.stereotype.Service
import org.springframework.transaction.annotation.Transactional
import java.security.MessageDigest
import java.time.Instant

/** One-use, owner-bound SHA-256 proof-of-work. A cost barrier, NOT a replacement for quotas. */
@Service
@Profile("server")
class ChallengeService(
    private val challenges: ChallengeRepository,
    private val limiter: RateLimiter,
    private val props: PlatformProperties,
) {
    @Transactional
    fun issue(ownerId: String): ChallengeDto {
        limiter.check("challenge-global", RateRule(GLOBAL_PER_MINUTE))
        limiter.check("challenge:$ownerId", RateRule(OWNER_PER_MINUTE))
        val id = newId()
        val salt = Secrets.token()
        val expiresAt = Instant.now().plusSeconds(TTL_SECONDS)
        challenges.insert(id, ownerId, salt, props.powDifficulty, expiresAt)
        return ChallengeDto(id, salt, props.powDifficulty, expiresAt, ALGORITHM)
    }

    /** Проверяет и гасит решение; без `requireChallenge` ничего не требует. */
    @Transactional
    fun consume(ownerId: String, id: String?, nonce: String?) {
        if (!props.requireChallenge) return
        if (id == null || nonce == null || !NONCE_PATTERN.matches(nonce)) badRequest("Нужно пройти проверку перед загрузкой")
        val challenge = challenges.lockById(id) ?: badRequest("Проверка истекла")
        if (challenge.ownerId != ownerId || challenge.used || challenge.expiresAt.isBefore(Instant.now())) {
            badRequest("Проверка истекла или уже использована")
        }
        val digest = MessageDigest.getInstance(ALGORITHM).digest("$id:${challenge.salt}:$nonce".toByteArray())
        if (leadingZeroBits(digest) < challenge.difficulty) badRequest("Некорректное подтверждение проверки")
        challenges.markUsed(id)
    }

    private fun leadingZeroBits(digest: ByteArray): Int {
        var bits = 0
        for (byte in digest) {
            val value = byte.toInt() and BYTE_MASK
            if (value == 0) {
                bits += Byte.SIZE_BITS
            } else {
                bits += Integer.numberOfLeadingZeros(value) - (Int.SIZE_BITS - Byte.SIZE_BITS)
                break
            }
        }
        return bits
    }

    private companion object {
        const val ALGORITHM = "SHA-256"
        const val TTL_SECONDS = 180L
        const val GLOBAL_PER_MINUTE = 100
        const val OWNER_PER_MINUTE = 10
        const val BYTE_MASK = 0xff
        val NONCE_PATTERN = Regex("[0-9]{1,16}")
    }
}
