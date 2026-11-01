package com.goatpacker.web.service.server

import com.goatpacker.web.dto.PasswordResetIssuedDto
import com.goatpacker.web.entity.AuditEntity
import com.goatpacker.web.entity.PrincipalKind
import com.goatpacker.web.entity.ResetEntity
import com.goatpacker.web.entity.Role
import com.goatpacker.web.exception.accessBlocked
import com.goatpacker.web.exception.badRequest
import com.goatpacker.web.exception.notFound
import com.goatpacker.web.repository.AuditRepository
import com.goatpacker.web.repository.PlatformRepository
import com.goatpacker.web.repository.PrincipalRepository
import com.goatpacker.web.repository.ResetRepository
import com.goatpacker.web.repository.SessionRepository
import com.goatpacker.web.security.Secrets
import com.goatpacker.web.util.SECONDS_PER_MINUTE
import org.springframework.context.annotation.Profile
import org.springframework.security.crypto.password.PasswordEncoder
import org.springframework.stereotype.Service
import org.springframework.transaction.annotation.Transactional
import java.time.Instant

/** Сброс пароля по одноразовой ссылке, которую администратор передаёт пользователю. */
@Service
@Profile("server")
class PasswordResetService(
    private val owners: PrincipalRepository,
    private val sessions: SessionRepository,
    private val resets: ResetRepository,
    private val platform: PlatformRepository,
    private val audit: AuditRepository,
    private val encoder: PasswordEncoder,
) {
    /** Выдаёт токен сброса (виден один раз) и отзывает все сессии пользователя. */
    @Transactional
    fun issue(adminId: String, targetId: String, reason: String): PasswordResetIssuedDto {
        platform.lockForUpdate()
        val target = owners.findByIdForUpdate(targetId) ?: notFound()
        if (target.kind != PrincipalKind.USER || target.role == Role.ADMIN) {
            badRequest("Сброс доступен только для обычного зарегистрированного пользователя")
        }
        val trimmedReason = AccountPolicy.requireReason(reason)
        target.resetRequired = true
        sessions.revokeAll(target.id)
        resets.deleteByOwner(target.id)
        val token = Secrets.token()
        resets.save(ResetEntity(Secrets.hash(token), target.id, Instant.now().plusSeconds(RESET_TTL_SECONDS)))
        audit.save(
            AuditEntity(actorId = adminId, targetId = target.id, action = AuditActions.PASSWORD_RESET_ISSUED, details = trimmedReason),
        )
        return PasswordResetIssuedDto(token, RESET_TTL_SECONDS)
    }

    @Transactional
    fun consume(token: String, newPassword: String) {
        platform.lockForUpdate()
        AccountPolicy.requireValidPassword(newPassword)
        val reset = resets.findByTokenHashForUpdate(Secrets.hash(token)) ?: badRequest("Ссылка сброса недействительна")
        if (reset.usedAt != null || reset.expiresAt.isBefore(Instant.now())) {
            badRequest("Ссылка сброса истекла или уже использована")
        }
        val owner = owners.findByIdForUpdate(reset.ownerId) ?: notFound()
        if (owner.blocked) accessBlocked()
        reset.usedAt = Instant.now()
        owner.passwordHash = encoder.encode(newPassword)
        owner.resetRequired = false
        sessions.revokeAll(owner.id)
        audit.save(AuditEntity(targetId = owner.id, action = AuditActions.PASSWORD_RESET_COMPLETED, details = "Sessions revoked"))
    }

    private companion object {
        /** Ссылка сброса действует 15 минут. */
        const val RESET_TTL_SECONDS = 15 * SECONDS_PER_MINUTE
    }
}
