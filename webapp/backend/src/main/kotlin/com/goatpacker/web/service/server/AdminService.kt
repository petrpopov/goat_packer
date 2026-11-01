package com.goatpacker.web.service.server

import com.goatpacker.web.config.PlatformProperties
import com.goatpacker.web.dto.AdminUserDto
import com.goatpacker.web.dto.AuditEventDto
import com.goatpacker.web.dto.SystemStateDto
import com.goatpacker.web.dto.UserPageDto
import com.goatpacker.web.dto.toDto
import com.goatpacker.web.entity.AuditEntity
import com.goatpacker.web.entity.PlatformEntity
import com.goatpacker.web.entity.PrincipalKind
import com.goatpacker.web.entity.QueueStatus
import com.goatpacker.web.entity.Role
import com.goatpacker.web.exception.badRequest
import com.goatpacker.web.exception.notFound
import com.goatpacker.web.repository.AuditRepository
import com.goatpacker.web.repository.PlatformRepository
import com.goatpacker.web.repository.PrincipalRepository
import com.goatpacker.web.repository.QueueJobRepository
import com.goatpacker.web.repository.QueueJobSqlRepository
import com.goatpacker.web.repository.SessionRepository
import com.goatpacker.web.repository.StorageUsageRepository
import com.goatpacker.web.repository.WorkerRepository
import org.springframework.context.annotation.Profile
import org.springframework.data.domain.PageRequest
import org.springframework.data.domain.Sort
import org.springframework.stereotype.Service
import org.springframework.transaction.annotation.Transactional
import java.time.Instant

/** Администрирование: пользователи, блокировки, параллелизм и журнал. */
@Service
@Profile("server")
class AdminService(
    private val owners: PrincipalRepository,
    private val sessions: SessionRepository,
    private val config: PlatformRepository,
    private val audit: AuditRepository,
    private val workers: WorkerRepository,
    private val jobs: QueueJobRepository,
    private val queueSql: QueueJobSqlRepository,
    private val storage: StorageUsageRepository,
    private val props: PlatformProperties,
) {
    /** Страница пользователей; [kind] — пусто (все), `GUEST` или `USER`, [query] — часть имени или id. */
    fun listUsers(kind: String, query: String, page: Int): UserPageDto {
        val filter = if (kind.isEmpty()) null else PrincipalKind.entries.find { it.name == kind }
        if ((kind.isNotEmpty() && filter == null) || query.length > MAX_QUERY_LENGTH || page !in Paging.PAGE_RANGE) {
            badRequest("Некорректный фильтр")
        }
        val pageable = PageRequest.of(page, USERS_PAGE_SIZE, Sort.by("createdAt").descending())
        val term = query.lowercase()
        val result = if (filter == null) owners.search(term, pageable) else owners.searchByKind(filter, term, pageable)
        val items = result.content.map {
            AdminUserDto(it.toDto(), it.createdAt, it.lastSeenAt, it.blockReason, it.resetRequired, storage.ownerBytes(it.id))
        }
        return UserPageDto(items, result.totalElements, page)
    }

    @Transactional
    fun setBlocked(adminId: String, targetId: String, blocked: Boolean, reason: String) {
        config.lockForUpdate()
        val target = owners.findByIdForUpdate(targetId) ?: notFound()
        if (target.id == adminId || target.role == Role.ADMIN) badRequest("Нельзя блокировать себя или администратора")
        val trimmedReason = AccountPolicy.requireReason(reason)
        target.blocked = blocked
        target.blockReason = if (blocked) trimmedReason else null
        if (blocked) {
            sessions.revokeAll(target.id)
            queueSql.requestCancelOfActiveJobs(targetId)
        }
        val action = if (blocked) AuditActions.ACCESS_BLOCKED else AuditActions.ACCESS_UNBLOCKED
        audit.save(AuditEntity(actorId = adminId, targetId = targetId, action = action, details = trimmedReason))
    }

    fun systemState(): SystemStateDto {
        val settings = config.findById(PlatformEntity.PLATFORM_CONFIG_ID).orElseThrow()
        val nodes = workers.findAll()
        val healthy = nodes.filter { it.lastSeenAt.isAfter(Instant.now().minusSeconds(WorkerNodeService.FRESH_SECONDS.toLong())) }
        val capacity = healthy.sumOf { it.capacity }
        return SystemStateDto(
            configuredParallelism = settings.maxParallel,
            effectiveParallelism = if (settings.paused) 0 else minOf(settings.maxParallel, capacity),
            lostJobs = jobs.countByStatus(QueueStatus.LOST),
            reportedActiveJobs = healthy.sumOf { it.activeCount },
            paused = settings.paused,
            activeJobs = jobs.countByStatusIn(QueueStatus.EXECUTING),
            queuedJobs = jobs.countByStatusIn(QueueStatus.WAITING),
            workers = nodes.map { it.toDto() },
            storageUsedBytes = storage.totalBytes(),
            storageLimitBytes = props.storageLimitBytes,
            apiAvailableProcessors = Runtime.getRuntime().availableProcessors(),
            apiHeapMaxBytes = Runtime.getRuntime().maxMemory(),
        )
    }

    @Transactional
    fun configureParallelism(adminId: String, maxParallel: Int, paused: Boolean, reason: String) {
        if (maxParallel !in 0..PlatformEntity.MAX_PARALLELISM || reason.trim().length !in AccountPolicy.REASON_LENGTH) {
            badRequest("Параллелизм 0–${PlatformEntity.MAX_PARALLELISM} и причина обязательны")
        }
        val setting = config.lockForUpdate()
        val previous = setting.maxParallel
        setting.maxParallel = maxParallel
        setting.paused = paused
        audit.save(
            AuditEntity(
                actorId = adminId,
                action = AuditActions.SYSTEM_LIMIT_UPDATED,
                details = "parallelism=$previous->$maxParallel paused=$paused reason=${reason.trim()}",
            ),
        )
    }

    fun auditLog(page: Int): List<AuditEventDto> {
        Paging.requirePage(page)
        return audit.findAllByOrderByCreatedAtDesc(PageRequest.of(page, USERS_PAGE_SIZE)).content.map { it.toDto() }
    }

    private companion object {
        const val USERS_PAGE_SIZE = 30
        const val MAX_QUERY_LENGTH = 100
    }
}
