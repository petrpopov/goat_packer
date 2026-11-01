package com.goatpacker.web.entity

import jakarta.persistence.Entity
import jakarta.persistence.Id
import jakarta.persistence.Table
import java.time.Instant

/** Сессия: в БД лежит только хеш токена из cookie. */
@Entity
@Table(name = "auth_sessions")
class SessionEntity(
    @Id var tokenHash: String = "",
    var ownerId: String = "",
    var expiresAt: Instant = Instant.now(),
    var revoked: Boolean = false,
)

/** Одноразовая ссылка сброса пароля, выданная администратором. */
@Entity
@Table(name = "password_resets")
class ResetEntity(
    @Id var tokenHash: String = "",
    var ownerId: String = "",
    var expiresAt: Instant = Instant.now(),
    var usedAt: Instant? = null,
)

/** Запись журнала действий администратора и системных событий. */
@Entity
@Table(name = "audit_events")
class AuditEntity(
    @Id var id: String = newId(),
    var actorId: String? = null,
    var targetId: String? = null,
    var action: String = "",
    var details: String = "",
    var createdAt: Instant = Instant.now(),
)
