package com.goatpacker.web.dto

import com.fasterxml.jackson.annotation.JsonUnwrapped
import com.goatpacker.web.entity.AuditEntity
import com.goatpacker.web.entity.PrincipalEntity
import com.goatpacker.web.entity.PrincipalKind
import com.goatpacker.web.entity.Role
import com.goatpacker.web.entity.WorkerEntity
import java.time.Instant

data class CredentialsRequest(val login: String = "", val password: String = "")

data class ResetPasswordRequest(val token: String = "", val password: String = "")

data class CsrfTokenDto(val token: String)

/** Кто сейчас работает с сервисом (ответ эндпоинтов auth). */
data class PrincipalDto(
    val id: String,
    val kind: PrincipalKind,
    val login: String?,
    val role: Role,
    val blocked: Boolean,
    val storageQuotaBytes: Long,
)

fun PrincipalEntity.toDto() = PrincipalDto(id, kind, login, role, blocked, storageQuota)

data class BlockRequest(val blocked: Boolean = true, val reason: String = "")

data class ReasonRequest(val reason: String = "")

data class SystemSettingsRequest(val maxParallel: Int = 1, val paused: Boolean = false, val reason: String = "")

/** Строка списка пользователей в админке: данные [PrincipalDto] плюс служебные поля. */
data class AdminUserDto(
    @get:JsonUnwrapped val principal: PrincipalDto,
    val createdAt: Instant,
    val lastSeenAt: Instant,
    val blockReason: String?,
    val resetRequired: Boolean,
    val storageUsedBytes: Long,
)

data class UserPageDto(val items: List<AdminUserDto>, val total: Long, val page: Int)

data class PasswordResetIssuedDto(val token: String, val expiresInSeconds: Long)

data class AuditEventDto(
    val id: String,
    val actorId: String?,
    val targetId: String?,
    val action: String,
    val details: String,
    val createdAt: Instant,
)

fun AuditEntity.toDto() = AuditEventDto(id, actorId, targetId, action, details, createdAt)

data class WorkerNodeDto(
    val id: String,
    val bootId: String,
    val cpuLimit: Double,
    val memoryLimitBytes: Long,
    val capacity: Int,
    val activeCount: Int,
    val lastSeenAt: Instant,
    val mode: String,
    val hostId: String,
)

fun WorkerEntity.toDto() = WorkerNodeDto(id, bootId, cpuLimit, memoryLimitBytes, capacity, activeCount, lastSeenAt, mode, hostId)

/** Состояние платформы для админки. */
data class SystemStateDto(
    val configuredParallelism: Int,
    val effectiveParallelism: Int,
    val lostJobs: Long,
    val reportedActiveJobs: Int,
    val paused: Boolean,
    val activeJobs: Long,
    val queuedJobs: Long,
    val workers: List<WorkerNodeDto>,
    val storageUsedBytes: Long,
    val storageLimitBytes: Long,
    val apiAvailableProcessors: Int,
    val apiHeapMaxBytes: Long,
)

/** Одноразовая задача на проверку «не бот» (proof-of-work). */
data class ChallengeDto(
    val id: String,
    val salt: String,
    val difficulty: Int,
    val expiresAt: Instant,
    val algorithm: String,
)
