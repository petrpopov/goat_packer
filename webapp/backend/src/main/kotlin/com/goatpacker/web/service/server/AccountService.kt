package com.goatpacker.web.service.server

import com.goatpacker.web.config.PlatformProperties
import com.goatpacker.web.dto.PrincipalDto
import com.goatpacker.web.dto.toDto
import com.goatpacker.web.entity.AuditEntity
import com.goatpacker.web.entity.PlatformEntity
import com.goatpacker.web.entity.PrincipalEntity
import com.goatpacker.web.entity.PrincipalKind
import com.goatpacker.web.entity.Role
import com.goatpacker.web.entity.USER_STORAGE_QUOTA_BYTES
import com.goatpacker.web.exception.ApiException
import com.goatpacker.web.exception.ErrorCode
import com.goatpacker.web.exception.accessBlocked
import com.goatpacker.web.exception.badRequest
import com.goatpacker.web.exception.notFound
import com.goatpacker.web.exception.quotaExceeded
import com.goatpacker.web.repository.AuditRepository
import com.goatpacker.web.repository.OwnerContentRepository
import com.goatpacker.web.repository.PlatformRepository
import com.goatpacker.web.repository.PrincipalRepository
import com.goatpacker.web.repository.SessionRepository
import com.goatpacker.web.security.Actor
import org.springframework.context.annotation.Profile
import org.springframework.http.HttpStatus
import org.springframework.security.crypto.password.PasswordEncoder
import org.springframework.stereotype.Service
import org.springframework.transaction.annotation.Transactional
import java.time.Instant

/** Результат входа: профиль и токен новой сессии (уходит клиенту в cookie). */
data class SignedIn(val user: PrincipalDto, val sessionToken: String)

/** Гости, регистрация, вход и стартовый администратор. */
@Service
@Profile("server")
class AccountService(
    private val owners: PrincipalRepository,
    private val sessions: SessionRepository,
    private val platform: PlatformRepository,
    private val audit: AuditRepository,
    private val content: OwnerContentRepository,
    private val sessionService: SessionService,
    private val encoder: PasswordEncoder,
    private val props: PlatformProperties,
) {
    fun currentUser(actor: Actor): PrincipalDto =
        owners.findById(actor.id).orElseThrow { unauthorized("Сессия не найдена") }.toDto()

    /** Создаёт администратора из настроек, если в сервисе ещё нет ни одного. */
    @Transactional
    fun bootstrapAdministrator() {
        platform.createIfAbsent(props.maxParallel.coerceIn(0, PlatformEntity.MAX_PARALLELISM))
        platform.lockForUpdate()
        if (props.adminLogin.isBlank()) return
        val login = AccountPolicy.normalizeLogin(props.adminLogin)
        if (owners.findByLogin(login) != null) return
        if (owners.count() > 0 && owners.countByRole(Role.ADMIN) > 0) return

        AccountPolicy.requireValidPassword(props.adminPassword)
        val admin = owners.save(
            PrincipalEntity(
                kind = PrincipalKind.USER,
                login = login,
                passwordHash = encoder.encode(props.adminPassword),
                role = Role.ADMIN,
                storageQuota = USER_STORAGE_QUOTA_BYTES,
            ),
        )
        audit.save(AuditEntity(targetId = admin.id, action = AuditActions.ADMIN_BOOTSTRAP, details = "Initial administrator created"))
    }

    @Transactional
    fun createGuest(): SignedIn {
        platform.lockForUpdate()
        requireRoomForNewPrincipal()
        val guest = owners.save(PrincipalEntity())
        return SignedIn(guest.toDto(), sessionService.open(guest))
    }

    /** Регистрация: гость становится пользователем вместе со своими задачами и файлами; без гостя заводится новый аккаунт. */
    @Transactional
    fun register(login: String, password: String, current: Actor?): SignedIn {
        platform.lockForUpdate()
        val normalized = AccountPolicy.normalizeLogin(login)
        AccountPolicy.requireValidPassword(password)
        if (owners.findByLogin(normalized) != null) badRequest("Это имя уже занято")

        val principal = if (current?.kind == PrincipalKind.GUEST) {
            owners.findByIdForUpdate(current.id) ?: notFound()
        } else {
            if (current != null) badRequest("Сначала выйдите из текущего аккаунта")
            requireRoomForNewPrincipal()
            PrincipalEntity()
        }
        if (principal.blocked) accessBlocked()
        principal.kind = PrincipalKind.USER
        principal.login = normalized
        principal.passwordHash = encoder.encode(password)
        principal.storageQuota = USER_STORAGE_QUOTA_BYTES
        owners.saveAndFlush(principal)
        sessions.revokeAll(principal.id)
        content.retainPermanently(principal.id)
        audit.save(AuditEntity(actorId = principal.id, targetId = principal.id, action = AuditActions.REGISTER, details = "Guest content retained"))
        return SignedIn(principal.toDto(), sessionService.open(principal))
    }

    @Transactional
    fun login(login: String, password: String): SignedIn {
        if (password.length > AccountPolicy.MAX_LOGIN_PASSWORD_LENGTH) badRequest("Некорректные данные входа")
        val principal = owners.findByLogin(AccountPolicy.loginKey(login))
        // Хеш считается и для несуществующего имени: по времени ответа нельзя понять, есть ли такой пользователь.
        val valid = encoder.matches(password, principal?.passwordHash ?: TIMING_DUMMY_HASH)
        if (principal == null || !valid || principal.resetRequired) {
            throw unauthorized("Неверные данные входа или требуется сброс пароля")
        }
        if (principal.blocked) accessBlocked()
        principal.lastSeenAt = Instant.now()
        return SignedIn(principal.toDto(), sessionService.open(principal))
    }

    private fun requireRoomForNewPrincipal() {
        if (owners.count() >= props.maxPrincipals) quotaExceeded("Лимит пользователей сервиса достигнут")
    }

    private fun unauthorized(message: String) = ApiException(HttpStatus.UNAUTHORIZED, ErrorCode.UNAUTHORIZED, message)

    private companion object {
        const val TIMING_DUMMY_HASH = "\$2a\$12\$Ywrkc.hMnqcA84Pd7qxSLukuwMfTbXwWSxJrKOOhQczRb0HMk1WRK"
    }
}

/** Действия журнала аудита (`audit_events.action`). */
object AuditActions {
    const val ADMIN_BOOTSTRAP = "ADMIN_BOOTSTRAP"
    const val REGISTER = "REGISTER"
    const val PASSWORD_RESET_ISSUED = "PASSWORD_RESET_ISSUED"
    const val PASSWORD_RESET_COMPLETED = "PASSWORD_RESET_COMPLETED"
    const val ACCESS_BLOCKED = "ACCESS_BLOCKED"
    const val ACCESS_UNBLOCKED = "ACCESS_UNBLOCKED"
    const val SYSTEM_LIMIT_UPDATED = "SYSTEM_LIMIT_UPDATED"
}
