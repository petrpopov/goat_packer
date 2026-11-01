package com.goatpacker.web.repository

import com.goatpacker.web.entity.PrincipalKind
import com.goatpacker.web.entity.QueueStatus
import org.springframework.context.annotation.Profile
import org.springframework.jdbc.core.JdbcTemplate
import org.springframework.stereotype.Repository

/** Удаление просроченных данных. Объекты хранилища отсюда не удаляются — только ставятся в [ObjectGcRepository]. */
@Repository
@Profile("server")
class RetentionRepository(private val jdbc: JdbcTemplate) {
    /** Завершённые задачи (без активных проверок) с истёкшим сроком хранения; проверки живут и умирают с родителем. */
    fun findExpiredJobIds(limit: Int): List<String> = jdbc.query(
        """select id from queue_jobs where parent_job_id is null and expires_at<now()
           and status not in ${QueueStatus.ACTIVE.sqlList()}
           and not exists(select 1 from queue_jobs c where c.parent_job_id=queue_jobs.id and c.status in ${QueueStatus.ACTIVE.sqlList()})
           limit $limit""",
    ) { rs, _ -> rs.getString(1) }

    fun deleteExpiredUploads() {
        jdbc.update("delete from uploads where expires_at<now()")
    }

    fun deleteOldDebugArtifacts(days: Int) {
        jdbc.update("delete from artifacts where debug=true and created_at<now()-interval '$days days'")
    }

    /** Блобы, на которые не ссылается ни загрузка, ни задача, ни артефакт. */
    fun findUnusedBlobs(limit: Int): List<UnusedBlob> = jdbc.query(
        """select id,object_key,size_bytes from blobs b where not exists(select 1 from uploads u where u.blob_id=b.id)
           and not exists(select 1 from queue_jobs j where j.input_blob_id=b.id)
           and not exists(select 1 from artifacts a where a.object_key=b.object_key) limit $limit""",
    ) { rs, _ -> UnusedBlob(rs.getString(1), rs.getString(2), rs.getLong(3)) }

    fun deleteBlob(id: String) {
        jdbc.update("delete from blobs where id=?", id)
    }

    fun deleteExpiredSessionsResetsAndGrants() {
        jdbc.update("delete from auth_sessions where expires_at<now()")
        jdbc.update("delete from password_resets where expires_at<now()")
        jdbc.update("delete from read_grants where expires_at<now()")
    }

    /** Гости, не появлявшиеся [idleDays] дней и ничем не владеющие. */
    fun deleteIdleGuests(idleDays: Int) {
        jdbc.update(
            """delete from principals p where p.kind='${PrincipalKind.GUEST.name}' and not p.blocked and p.last_seen_at<now()-interval '$idleDays days'
               and not exists(select 1 from auth_sessions s where s.owner_id=p.id)
               and not exists(select 1 from uploads u where u.owner_id=p.id)
               and not exists(select 1 from queue_jobs j where j.owner_id=p.id)
               and not exists(select 1 from read_grants g where g.owner_id=p.id)
               and not exists(select 1 from bot_challenges c where c.owner_id=p.id)""",
        )
    }

    fun deleteExpiredChallenges() {
        jdbc.update("delete from bot_challenges where expires_at<now()")
    }

    fun deleteOldComputeBudgets(days: Int) {
        jdbc.update("delete from compute_budgets where day<current_date-$days")
    }

    fun deleteOldRequestLimits(days: Int) {
        jdbc.update("delete from request_limits where window_start<now()-interval '$days days'")
    }
}

data class UnusedBlob(val id: String, val objectKey: String, val sizeBytes: Long)
