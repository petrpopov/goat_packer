package com.goatpacker.web.service.server

import com.goatpacker.web.entity.ArtifactEntity
import com.goatpacker.web.entity.QueueJobEntity
import com.goatpacker.web.exception.expired
import com.goatpacker.web.exception.notFound
import com.goatpacker.web.repository.ArtifactRepository
import com.goatpacker.web.repository.PlatformRepository
import com.goatpacker.web.repository.PrincipalRepository
import com.goatpacker.web.repository.QueueJobRepository
import com.goatpacker.web.repository.ReadGrantRepository
import com.goatpacker.web.security.Secrets
import org.springframework.context.annotation.Profile
import org.springframework.stereotype.Service
import org.springframework.transaction.annotation.Transactional
import java.time.Instant

/** Кто и как может видеть задачу: владелец, обладатель ссылки (токен чтения) или получатель выданного доступа. */
@Service
@Profile("server")
class JobAccessService(
    private val jobs: QueueJobRepository,
    private val owners: PrincipalRepository,
    private val artifacts: ArtifactRepository,
    private val grants: ReadGrantRepository,
    private val platform: PlatformRepository,
) {
    /** Задача владельца [ownerId]; чужая или внутренняя проверка — «не найдено». */
    fun findOwned(id: String, ownerId: String): QueueJobEntity =
        jobs.findById(id).orElse(null)?.takeIf { it.ownerId == ownerId }?.also(::requireNotExpired) ?: notFound()

    /** Задача, доступная читателю [viewerId]: своя, по выданному доступу или по токену ссылки. Проверки скрыты. */
    fun findReadable(id: String, viewerId: String, token: String?): QueueJobEntity {
        val job = jobs.findById(id).orElse(null)?.takeIf { it.parentJobId == null } ?: notFound()
        val granted = grants.hasActive(viewerId, id)
        if (job.ownerId != viewerId && !granted && !matchesReadToken(job, token)) notFound()
        if (owners.findById(job.ownerId).orElse(null)?.blocked != false) notFound()
        requireNotExpired(job)
        return job
    }

    /** Открывает задачу по токену ссылки для [viewerId] насовсем (пока не истечёт срок задачи). */
    @Transactional
    fun grantAccess(id: String, viewerId: String, token: String) {
        val job = findReadable(id, viewerId, token)
        val expiresAt = job.expiresAt ?: Instant.now().plusSeconds(RetentionPolicy.GUEST_CONTENT_TTL_SECONDS)
        grants.grant(viewerId, id, expiresAt)
    }

    /** Новая ссылка для чтения: прежняя ссылка и выданные по ней доступы перестают работать. */
    @Transactional
    fun createShareLink(id: String, ownerId: String): String {
        platform.lockForUpdate()
        val job = findOwned(id, ownerId)
        grants.revokeAll(id)
        val token = Secrets.token()
        job.readTokenHash = Secrets.hash(token)
        return token
    }

    /** Дочерняя проверка родителя (null — проверки нет); доступ уже проверен по родителю. */
    fun checkOf(parent: QueueJobEntity): QueueJobEntity? =
        parent.checkJobId?.let { jobs.findById(it).orElse(null) }?.takeIf { it.parentJobId == parent.id }

    /** Артефакт, который можно отдать по id родителя: его собственный или последней проверки. */
    fun findArtifact(parent: QueueJobEntity, artifactId: String): ArtifactEntity? =
        artifacts.findById(artifactId).orElse(null)?.takeIf { it.jobId == parent.id || it.jobId == checkOf(parent)?.id }

    private fun matchesReadToken(job: QueueJobEntity, token: String?): Boolean =
        token != null && token.length <= MAX_TOKEN_LENGTH && job.readTokenHash == Secrets.hash(token)

    private fun requireNotExpired(job: QueueJobEntity) {
        if (job.expiresAt?.isBefore(Instant.now()) == true) expired("Срок хранения задачи истёк")
    }

    private companion object {
        const val MAX_TOKEN_LENGTH = 100
    }
}
