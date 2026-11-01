package com.goatpacker.web.repository

import com.goatpacker.web.entity.ArtifactEntity
import com.goatpacker.web.entity.BlobEntity
import com.goatpacker.web.entity.PlatformEntity
import com.goatpacker.web.entity.QueueJobEntity
import com.goatpacker.web.entity.QueueStatus
import com.goatpacker.web.entity.UploadEntity
import com.goatpacker.web.entity.UploadState
import com.goatpacker.web.entity.WorkerEntity
import jakarta.persistence.LockModeType
import org.springframework.context.annotation.Profile
import org.springframework.data.domain.Page
import org.springframework.data.domain.Pageable
import org.springframework.data.jpa.repository.JpaRepository
import org.springframework.data.jpa.repository.Lock
import org.springframework.data.jpa.repository.Modifying
import org.springframework.data.jpa.repository.Query
import org.springframework.data.repository.query.Param
import org.springframework.transaction.annotation.Transactional

@Profile("server")
interface PlatformRepository : JpaRepository<PlatformEntity, Int> {
    /** Глобальный мьютекс платформы: мутации очереди, загрузок и аккаунтов сериализуются по этой строке. */
    @Lock(LockModeType.PESSIMISTIC_WRITE)
    @Query("select p from PlatformEntity p where p.id=1")
    fun lockForUpdate(): PlatformEntity

    /** Создаёт строку настроек при первом запуске; существующую не трогает. */
    @Transactional
    @Modifying
    @Query(
        "insert into platform_config(id,max_parallel,paused,version) values (1,:maxParallel,false,0) on conflict do nothing",
        nativeQuery = true,
    )
    fun createIfAbsent(@Param("maxParallel") maxParallel: Int): Int
}

@Profile("server")
interface BlobRepository : JpaRepository<BlobEntity, String> {
    fun findBySha256AndSizeBytes(sha256: String, sizeBytes: Long): BlobEntity?

    fun findByObjectKey(objectKey: String): BlobEntity?
}

@Profile("server")
interface UploadRepository : JpaRepository<UploadEntity, String> {
    fun findAllByOwnerId(ownerId: String): List<UploadEntity>

    fun countByStateIn(states: Collection<UploadState>): Long
}

@Profile("server")
interface QueueJobRepository : JpaRepository<QueueJobEntity, String> {
    fun findByOwnerIdAndRequestKey(ownerId: String, requestKey: String): QueueJobEntity?

    fun findAllByParentJobId(parentJobId: String): List<QueueJobEntity>

    fun findAllByOwnerIdAndParentJobIdIsNull(ownerId: String, pageable: Pageable): Page<QueueJobEntity>

    fun findAllByStatusOrderByCreatedAtAscIdAsc(status: QueueStatus): List<QueueJobEntity>

    fun findAllByWorkerIdAndStatusIn(workerId: String, statuses: Collection<QueueStatus>): List<QueueJobEntity>

    fun countByStatus(status: QueueStatus): Long

    fun countByStatusIn(statuses: Collection<QueueStatus>): Long

    fun countByOwnerIdAndStatusIn(ownerId: String, statuses: Collection<QueueStatus>): Long

    fun countByWorkerIdAndStatusIn(workerId: String, statuses: Collection<QueueStatus>): Long

    @Lock(LockModeType.PESSIMISTIC_WRITE)
    @Query("select j from QueueJobEntity j where j.id=:id")
    fun findByIdForUpdate(@Param("id") id: String): QueueJobEntity?
}

@Profile("server")
interface ArtifactRepository : JpaRepository<ArtifactEntity, String> {
    fun findAllByJobId(jobId: String): List<ArtifactEntity>
}

@Profile("server")
interface WorkerRepository : JpaRepository<WorkerEntity, String> {
    /** Суммарная ёмкость исполнителей, подавших признак жизни не позже [freshSeconds] секунд назад. */
    @Query(
        "select coalesce(sum(capacity),0) from worker_nodes where last_seen_at>now()-(:freshSeconds * interval '1 second')",
        nativeQuery = true,
    )
    fun sumFreshCapacity(@Param("freshSeconds") freshSeconds: Int): Int

    @Query(
        "select capacity from worker_nodes where id=:id and last_seen_at>now()-(:freshSeconds * interval '1 second')",
        nativeQuery = true,
    )
    fun findFreshCapacity(@Param("id") id: String, @Param("freshSeconds") freshSeconds: Int): Int?

    fun countByIdAndBootId(id: String, bootId: String): Long

    /** Регистрирует новую загрузку исполнителя: ёмкость 0, пока старые процессы не подтверждены остановленными. */
    @Transactional
    @Modifying
    @Query(
        """insert into worker_nodes(id,boot_id,cpu_limit,memory_limit_bytes,capacity,active_count,last_seen_at,mode,host_id)
           values (:id,:bootId,0,0,0,0,now(),:mode,:hostId)
           on conflict(id) do update set boot_id=excluded.boot_id,capacity=0,last_seen_at=now(),host_id=excluded.host_id""",
        nativeQuery = true,
    )
    fun register(
        @Param("id") id: String,
        @Param("bootId") bootId: String,
        @Param("mode") mode: String,
        @Param("hostId") hostId: String,
    ): Int

    /** Признак жизни; 0 строк — загрузка исполнителя вытеснена более новой. */
    @Transactional
    @Modifying
    @Query(
        """update worker_nodes set cpu_limit=:cpu,memory_limit_bytes=:memory,capacity=:capacity,active_count=:active,last_seen_at=now()
           where id=:id and boot_id=:bootId""",
        nativeQuery = true,
    )
    fun pulse(
        @Param("id") id: String,
        @Param("bootId") bootId: String,
        @Param("cpu") cpuLimit: Double,
        @Param("memory") memoryLimitBytes: Long,
        @Param("capacity") capacity: Int,
        @Param("active") activeCount: Int,
    ): Int

    /** Исполнитель остановлен штатно: ёмкость 0 и «давно не виден», чтобы слот освободился сразу. */
    @Transactional
    @Modifying
    @Query(
        """update worker_nodes set capacity=0,active_count=0,last_seen_at=now()-(:staleSeconds * interval '1 second')
           where id=:id and boot_id=:bootId""",
        nativeQuery = true,
    )
    fun markStopped(@Param("id") id: String, @Param("bootId") bootId: String, @Param("staleSeconds") staleSeconds: Int): Int
}
