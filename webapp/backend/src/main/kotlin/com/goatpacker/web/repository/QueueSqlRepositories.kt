package com.goatpacker.web.repository

import com.goatpacker.web.entity.QueueStatus
import com.goatpacker.web.entity.UploadState
import com.goatpacker.web.model.JobKind
import org.springframework.context.annotation.Profile
import org.springframework.jdbc.core.JdbcTemplate
import org.springframework.stereotype.Repository
import java.sql.Timestamp
import java.time.Instant

/** Запросы очереди, которые не выражаются Spring Data: позиции, медианы, выбор следующей задачи. */
@Repository
@Profile("server")
class QueueJobSqlRepository(private val jdbc: JdbcTemplate) {
    /** Сколько ожидающих задач стоят в очереди раньше [id] (порядок: время создания, затем id). */
    fun waitingAhead(createdAt: Instant, id: String): Long = jdbc.queryForObject(
        "select count(*) from queue_jobs where status in ${QueueStatus.WAITING.sqlList()} and (created_at,id)<(?,?)",
        Long::class.java,
        Timestamp.from(createdAt),
        id,
    )!!

    /** Длительность последних удачных задач такого же вида на таком же принтере. */
    fun recentSuccessDurations(kind: JobKind, profileId: String): DurationStats {
        val row = jdbc.queryForMap(
            """select count(*) as n,
               percentile_cont(0.5) within group(order by extract(epoch from finished_at-started_at)) as median
               from (select started_at,finished_at from queue_jobs
               where status='${QueueStatus.SUCCEEDED.name}' and kind=? and profile_id=? and started_at is not null
               order by finished_at desc limit $DURATION_SAMPLE_SIZE) sample""",
            kind.name,
            profileId,
        )
        return DurationStats((row["n"] as Number).toInt(), (row["median"] as Number?)?.toDouble())
    }

    /** Самая старая ожидающая задача, которую можно взять: не отменена, владелец не заблокирован и ничего не запускал. */
    fun lockNextClaimableJobId(): String? = jdbc.query(
        """select j.id from queue_jobs j join principals p on p.id=j.owner_id
           where j.status in ${QueueStatus.WAITING.sqlList()} and j.available_at<=now() and not j.cancel_requested and not p.blocked
           and not exists(select 1 from queue_jobs r where r.owner_id=j.owner_id and r.status in ${QueueStatus.OCCUPYING_SLOT.sqlList()})
           order by j.created_at,j.id for update of j skip locked limit 1""",
    ) { rs, _ -> rs.getString(1) }.firstOrNull()

    /** Помечает все активные задачи владельца на отмену (блокировка аккаунта). */
    fun requestCancelOfActiveJobs(ownerId: String) {
        jdbc.update(
            "update queue_jobs set cancel_requested=true where owner_id=? and status in ${QueueStatus.ACTIVE.sqlList()}",
            ownerId,
        )
    }

    /** Срок хранения проверки продлевает срок родителя, если у родителя он вообще есть. */
    fun extendExpiry(jobId: String, expiresAt: Instant) {
        jdbc.update(
            "update queue_jobs set expires_at=greatest(expires_at,?) where id=? and expires_at is not null",
            Timestamp.from(expiresAt),
            jobId,
        )
    }

    fun deleteJob(id: String) {
        jdbc.update("delete from queue_jobs where id=?", id)
    }

    private companion object {
        /** По скольким последним удачным задачам считается медиана времени выполнения. */
        const val DURATION_SAMPLE_SIZE = 50
    }
}

/** Медиана длительности выполнения (секунды) по [samples] последним удачным задачам. */
data class DurationStats(val samples: Int, val medianSeconds: Double?)

/** Попытки выполнения задачи (`job_attempts`): по одной на каждый захват задачи исполнителем. */
@Repository
@Profile("server")
class JobAttemptRepository(private val jdbc: JdbcTemplate) {
    fun start(id: String, jobId: String, fence: Long, workerId: String, bootId: String) {
        jdbc.update(
            "insert into job_attempts(id,job_id,fence,worker_id,boot_id,started_at,status) values (?,?,?,?,?,now(),'${QueueStatus.RUNNING.name}')",
            id,
            jobId,
            fence,
            workerId,
            bootId,
        )
    }

    fun finish(jobId: String, fence: Long, status: QueueStatus, message: String?) {
        jdbc.update(
            "update job_attempts set status=?,finished_at=now(),message=? where job_id=? and fence=?",
            status.name,
            message,
            jobId,
            fence,
        )
    }
}

/** Выданные доступы к чужим задачам по ссылке (`read_grants`). */
@Repository
@Profile("server")
class ReadGrantRepository(private val jdbc: JdbcTemplate) {
    fun hasActive(ownerId: String, jobId: String): Boolean = jdbc.queryForObject(
        "select count(*) from read_grants where owner_id=? and job_id=? and expires_at>now()",
        Long::class.java,
        ownerId,
        jobId,
    )!! > 0

    fun grant(ownerId: String, jobId: String, expiresAt: Instant) {
        jdbc.update(
            """insert into read_grants(owner_id,job_id,expires_at) values (?,?,?)
               on conflict(owner_id,job_id) do update set expires_at=excluded.expires_at""",
            ownerId,
            jobId,
            Timestamp.from(expiresAt),
        )
    }

    fun revokeAll(jobId: String) {
        jdbc.update("delete from read_grants where job_id=?", jobId)
    }
}

/** Содержимое владельца, которое перестаёт истекать при регистрации гостя. */
@Repository
@Profile("server")
class OwnerContentRepository(private val jdbc: JdbcTemplate) {
    fun retainPermanently(ownerId: String) {
        jdbc.update("delete from read_grants where job_id in (select id from queue_jobs where owner_id=?)", ownerId)
        jdbc.update("update queue_jobs set expires_at=null,read_token_hash=null where owner_id=?", ownerId)
        jdbc.update("update uploads set expires_at=null where owner_id=? and state='${UploadState.READY.name}'", ownerId)
    }
}
