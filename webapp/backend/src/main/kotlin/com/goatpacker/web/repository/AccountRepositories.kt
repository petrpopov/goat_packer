package com.goatpacker.web.repository

import com.goatpacker.web.entity.AuditEntity
import com.goatpacker.web.entity.PrincipalEntity
import com.goatpacker.web.entity.PrincipalKind
import com.goatpacker.web.entity.ResetEntity
import com.goatpacker.web.entity.Role
import com.goatpacker.web.entity.SessionEntity
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
import java.time.Instant

@Profile("server")
interface PrincipalRepository : JpaRepository<PrincipalEntity, String> {
    fun findByLogin(login: String): PrincipalEntity?

    fun countByRole(role: Role): Long

    @Lock(LockModeType.PESSIMISTIC_WRITE)
    @Query("select p from PrincipalEntity p where p.id=:id")
    fun findByIdForUpdate(@Param("id") id: String): PrincipalEntity?

    /** Пользователи, чьё имя (или id гостя) содержит [term] (в нижнем регистре). */
    @Query("select p from PrincipalEntity p where lower(coalesce(p.login,p.id)) like concat('%',:term,'%')")
    fun search(@Param("term") term: String, pageable: Pageable): Page<PrincipalEntity>

    @Query("select p from PrincipalEntity p where p.kind=:kind and lower(coalesce(p.login,p.id)) like concat('%',:term,'%')")
    fun searchByKind(@Param("kind") kind: PrincipalKind, @Param("term") term: String, pageable: Pageable): Page<PrincipalEntity>

    /** Обновляет время последнего визита не чаще раза в минуту (защита от лишних записей на каждый запрос). */
    @Transactional
    @Modifying
    @Query(
        "update principals set last_seen_at=now() where id=:id and last_seen_at<now()-interval '1 minute'",
        nativeQuery = true,
    )
    fun touchLastSeen(@Param("id") id: String): Int
}

@Profile("server")
interface SessionRepository : JpaRepository<SessionEntity, String> {
    fun countByOwnerId(ownerId: String): Long

    @Transactional
    @Modifying
    @Query("update SessionEntity s set s.revoked=true where s.ownerId=:owner")
    fun revokeAll(@Param("owner") owner: String): Int

    @Transactional
    @Modifying
    @Query("delete from SessionEntity s where s.ownerId=:owner and (s.revoked=true or s.expiresAt<:now)")
    fun deleteStale(@Param("owner") owner: String, @Param("now") now: Instant = Instant.now()): Int

    @Transactional
    @Modifying
    @Query("update SessionEntity s set s.expiresAt=:expiresAt where s.tokenHash=:hash and s.revoked=false")
    fun extend(@Param("hash") tokenHash: String, @Param("expiresAt") expiresAt: Instant): Int
}

@Profile("server")
interface ResetRepository : JpaRepository<ResetEntity, String> {
    @Lock(LockModeType.PESSIMISTIC_WRITE)
    @Query("select r from ResetEntity r where r.tokenHash=:hash")
    fun findByTokenHashForUpdate(@Param("hash") hash: String): ResetEntity?

    @Modifying
    @Query("delete from ResetEntity r where r.ownerId=:owner")
    fun deleteByOwner(@Param("owner") owner: String)
}

@Profile("server")
interface AuditRepository : JpaRepository<AuditEntity, String> {
    fun findAllByOrderByCreatedAtDesc(pageable: Pageable): Page<AuditEntity>
}
