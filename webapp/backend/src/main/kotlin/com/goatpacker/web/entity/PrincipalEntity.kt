package com.goatpacker.web.entity

import jakarta.persistence.Entity
import jakarta.persistence.EnumType
import jakarta.persistence.Enumerated
import jakarta.persistence.Id
import jakarta.persistence.Table
import jakarta.persistence.Version
import java.time.Instant
import java.util.UUID

fun newId(): String = UUID.randomUUID().toString()

/** Квота хранения гостя, байт. */
const val GUEST_STORAGE_QUOTA_BYTES = 150L * 1024 * 1024

/** Квота хранения зарегистрированного пользователя, байт. */
const val USER_STORAGE_QUOTA_BYTES = 1024L * 1024 * 1024

/** Владелец данных: гость или пользователь (администратор — пользователь с ролью ADMIN). */
@Entity
@Table(name = "principals")
class PrincipalEntity(
    @Id var id: String = newId(),
    @Enumerated(EnumType.STRING) var kind: PrincipalKind = PrincipalKind.GUEST,
    var login: String? = null,
    var passwordHash: String? = null,
    @Enumerated(EnumType.STRING) var role: Role = Role.USER,
    var blocked: Boolean = false,
    var blockReason: String? = null,
    var resetRequired: Boolean = false,
    var createdAt: Instant = Instant.now(),
    var lastSeenAt: Instant = Instant.now(),
    var storageQuota: Long = GUEST_STORAGE_QUOTA_BYTES,
    @Version var version: Long = 0,
)
