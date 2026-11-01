package com.goatpacker.web.service.server

import com.goatpacker.web.entity.PrincipalEntity
import com.goatpacker.web.entity.SessionEntity
import com.goatpacker.web.exception.ApiException
import com.goatpacker.web.exception.ErrorCode
import com.goatpacker.web.repository.PrincipalRepository
import com.goatpacker.web.repository.SessionRepository
import com.goatpacker.web.security.Actor
import com.goatpacker.web.security.SESSION_SECONDS
import com.goatpacker.web.security.Secrets
import com.goatpacker.web.util.SECONDS_PER_DAY
import com.goatpacker.web.util.SECONDS_PER_MINUTE
import org.springframework.context.annotation.Profile
import org.springframework.http.HttpStatus
import org.springframework.stereotype.Service
import org.springframework.transaction.annotation.Transactional
import java.time.Instant

/** Сессии по cookie: проверка, скользящее продление, выдача и отзыв. В БД лежат только хеши токенов. */
@Service
@Profile("server")
class SessionService(
    private val owners: PrincipalRepository,
    private val sessions: SessionRepository,
) {
    /**
     * Сессия скользящая: живёт [SESSION_SECONDS] с последнего обращения, а не с создания. С жёсткими 7 днями гость,
     * открывший сайт ровно через неделю, получал 401 посреди работы («Проверить нарезкой»), а следующая загрузка
     * страницы молча заводила нового гостя — без доступа к его задачам. Продлевается не чаще раза в сутки;
     * [renewed] вызывается, когда браузеру нужно переписать cookie с новым сроком.
     */
    fun authenticate(token: String?, renewed: () -> Unit = {}): Actor? {
        if (token == null || token.length > MAX_TOKEN_LENGTH) return null
        val session = sessions.findById(Secrets.hash(token)).orElse(null) ?: return null
        val owner = owners.findById(session.ownerId).orElse(null) ?: return null
        if (owner.blocked) {
            throw ApiException(HttpStatus.FORBIDDEN, ErrorCode.BLOCKED, "Доступ заблокирован администратором")
        }
        val now = Instant.now()
        if (session.revoked || session.expiresAt.isBefore(now) || owner.resetRequired) return null

        if (owner.lastSeenAt.isBefore(now.minusSeconds(LAST_SEEN_GRANULARITY_SECONDS))) owners.touchLastSeen(owner.id)
        if (session.expiresAt.isBefore(now.plusSeconds(SESSION_SECONDS - RENEW_INTERVAL_SECONDS))) {
            sessions.extend(session.tokenHash, now.plusSeconds(SESSION_SECONDS))
            renewed()
        }
        return Actor(owner.id, owner.role, owner.kind, owner.login)
    }

    /** Заводит сессию владельца и возвращает её токен (в БД попадает только хеш). */
    @Transactional
    fun open(owner: PrincipalEntity): String {
        // Bound session count without exposing any token to logs.
        sessions.deleteStale(owner.id)
        if (sessions.countByOwnerId(owner.id) >= MAX_SESSIONS_PER_OWNER) sessions.revokeAll(owner.id)
        val token = Secrets.token()
        sessions.save(SessionEntity(Secrets.hash(token), owner.id, Instant.now().plusSeconds(SESSION_SECONDS)))
        return token
    }

    @Transactional
    fun close(token: String?) {
        token?.let { sessions.findById(Secrets.hash(it)).orElse(null) }?.revoked = true
    }

    private companion object {
        const val MAX_TOKEN_LENGTH = 100
        const val MAX_SESSIONS_PER_OWNER = 10
        const val LAST_SEEN_GRANULARITY_SECONDS = SECONDS_PER_MINUTE
        const val RENEW_INTERVAL_SECONDS = SECONDS_PER_DAY
    }
}
