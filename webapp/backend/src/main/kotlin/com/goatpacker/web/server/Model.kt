package com.goatpacker.web.server

import jakarta.persistence.*
import org.springframework.context.annotation.Profile
import org.springframework.data.domain.Page
import org.springframework.data.domain.Pageable
import org.springframework.data.jpa.repository.*
import org.springframework.data.jpa.repository.Query
import org.springframework.data.repository.query.Param
import java.time.Instant
import java.util.UUID

fun newId(): String = UUID.randomUUID().toString()

@Entity @Table(name = "principals")
class PrincipalEntity(
    @Id var id: String = newId(), var kind: String = "GUEST", var login: String? = null,
    var passwordHash: String? = null, var role: String = "USER", var blocked: Boolean = false,
    var blockReason: String? = null, var resetRequired: Boolean = false,
    var createdAt: Instant = Instant.now(), var lastSeenAt: Instant = Instant.now(),
    var storageQuota: Long = 150L * 1024 * 1024,
    @Version var version: Long = 0,
)
@Entity @Table(name="auth_sessions")
class SessionEntity(@Id var tokenHash: String = "", var ownerId: String = "", var expiresAt: Instant = Instant.now(), var revoked: Boolean = false)
@Entity @Table(name="password_resets")
class ResetEntity(@Id var tokenHash: String = "", var ownerId: String = "", var expiresAt: Instant = Instant.now(), var usedAt: Instant? = null)
@Entity @Table(name="platform_config")
class PlatformEntity(@Id var id: Int = 1, var maxParallel: Int = 1, var paused: Boolean = false, @Version var version: Long = 0)
@Entity @Table(name="blobs")
class BlobEntity(@Id var id: String = newId(), var sha256: String = "", var sizeBytes: Long = 0, var objectKey: String = "", var createdAt: Instant = Instant.now())
@Entity @Table(name="uploads")
class UploadEntity(
    @Id var id: String = newId(), var ownerId: String = "", var originalName: String = "", var expectedBytes: Long = 0,
    var state: String = "UPLOADING", var blobId: String? = null, var printerModel: String? = null, var profileId: String? = null,
    var createdAt: Instant = Instant.now(), var expiresAt: Instant? = Instant.now().plusSeconds(900), @Version var version: Long = 0,
)
@Entity @Table(name="queue_jobs")
class QueueJobEntity(
    @Id var id: String = newId(), var ownerId: String = "", var kind: String = "PACK", var status: String = "QUEUED",
    var inputBlobId: String = "", var originalName: String = "", var profileId: String = "",
    @Column(columnDefinition="text") var profileJson: String = "{}",
    @Column(columnDefinition="text") var paramsJson: String = "{}",
    var requestKey: String = "", var requestHash: String = "", var readTokenHash: String? = null,
    var phase: String = "upload", var progress: Int = 0, var message: String? = null, var error: String? = null,
    @Column(columnDefinition="text") var resultJson: String? = null,
    var createdAt: Instant = Instant.now(), var startedAt: Instant? = null, var finishedAt: Instant? = null, var expiresAt: Instant? = null,
    var workerId: String? = null, var fence: Long = 0, var leaseUntil: Instant? = null, var deadlineAt: Instant? = null,
    var cancelRequested: Boolean = false, var reservedBytes: Long = 0,
    var attemptCount: Int = 0, var availableAt: Instant = Instant.now(), var computeReservedSeconds: Long = 0,
    var budgetDay: java.time.LocalDate? = null, var runnerImage: String? = null, @Version var version: Long = 0,
)
@Entity @Table(name="artifacts")
class ArtifactEntity(
    @Id var id: String = newId(), var jobId: String = "", var name: String = "", var downloadName: String = "",
    var objectKey: String = "", var sizeBytes: Long = 0, var debug: Boolean = false, var createdAt: Instant = Instant.now(),
)
@Entity @Table(name="worker_nodes")
class WorkerEntity(
    @Id var id: String = "", var bootId: String = "", var cpuLimit: Double = 0.0, var memoryLimitBytes: Long = 0,
    var capacity: Int = 0, var activeCount: Int = 0, var lastSeenAt: Instant = Instant.now(), var mode: String = "docker", var hostId: String = "",
)
@Entity @Table(name="audit_events")
class AuditEntity(@Id var id: String = newId(), var actorId: String? = null, var targetId: String? = null, var action: String = "", var details: String = "", var createdAt: Instant = Instant.now())

@Profile("server") interface PrincipalRepository: JpaRepository<PrincipalEntity,String> {
    fun findByLogin(login: String): PrincipalEntity?
    @Lock(LockModeType.PESSIMISTIC_WRITE) @Query("select p from PrincipalEntity p where p.id=:id")
    fun locked(@Param("id") id: String): PrincipalEntity?
    @Query("select p from PrincipalEntity p where (:kind='' or p.kind=:kind) and (:term='' or lower(coalesce(p.login,p.id)) like concat('%',:term,'%'))")
    fun search(@Param("kind") kind: String, @Param("term") term: String, pageable: Pageable): Page<PrincipalEntity>
}
@Profile("server") interface SessionRepository: JpaRepository<SessionEntity,String> {
    @Modifying @Query("update SessionEntity s set s.revoked=true where s.ownerId=:owner") fun revoke(@Param("owner") owner: String): Int
}
@Profile("server") interface ResetRepository: JpaRepository<ResetEntity,String> {
    @Lock(LockModeType.PESSIMISTIC_WRITE) @Query("select r from ResetEntity r where r.tokenHash=:hash") fun locked(@Param("hash") hash: String): ResetEntity?
    @Modifying @Query("delete from ResetEntity r where r.ownerId=:owner") fun removeFor(@Param("owner") owner: String)
}
@Profile("server") interface PlatformRepository: JpaRepository<PlatformEntity,Int> {
    @Lock(LockModeType.PESSIMISTIC_WRITE) @Query("select p from PlatformEntity p where p.id=1") fun locked(): PlatformEntity
}
@Profile("server") interface BlobRepository: JpaRepository<BlobEntity,String> { fun findBySha256AndSizeBytes(sha256:String,sizeBytes:Long): BlobEntity? }
@Profile("server") interface UploadRepository: JpaRepository<UploadEntity,String> { fun findAllByOwnerId(ownerId:String): List<UploadEntity> }
@Profile("server") interface QueueJobRepository: JpaRepository<QueueJobEntity,String> {
    fun findByOwnerIdAndRequestKey(ownerId:String,requestKey:String): QueueJobEntity?
    fun findAllByOwnerId(ownerId:String, pageable:Pageable): Page<QueueJobEntity>
    fun findAllByStatusOrderByCreatedAtAscIdAsc(status:String): List<QueueJobEntity>
    @Lock(LockModeType.PESSIMISTIC_WRITE) @Query("select j from QueueJobEntity j where j.id=:id") fun locked(@Param("id") id:String):QueueJobEntity?
}
@Profile("server") interface ArtifactRepository: JpaRepository<ArtifactEntity,String> { fun findAllByJobId(jobId:String): List<ArtifactEntity> }
@Profile("server") interface WorkerRepository: JpaRepository<WorkerEntity,String>
@Profile("server") interface AuditRepository: JpaRepository<AuditEntity,String> { fun findAllByOrderByCreatedAtDesc(pageable:Pageable):Page<AuditEntity> }
