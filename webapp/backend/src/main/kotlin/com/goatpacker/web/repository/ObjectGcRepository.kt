package com.goatpacker.web.repository

import com.goatpacker.web.entity.UploadState
import org.springframework.context.annotation.Profile
import org.springframework.jdbc.core.JdbcTemplate
import org.springframework.stereotype.Repository
import org.springframework.transaction.annotation.Transactional

/**
 * Очередь удаления объектов хранилища (`object_gc`): ключи неизменяемы, поэтому объект удаляется только
 * после коммита БД и только если на него никто не ссылается. Строка с `not_before` в будущем — резерв под
 * объект, который ещё заливают: он защищает от GC на время заливки.
 */
@Repository
@Profile("server")
class ObjectGcRepository(private val jdbc: JdbcTemplate) {
    /** Резервирует [key] под заливку: если публикация не случится, объект уберёт GC через [STAGING_HOURS] часа. */
    fun stage(key: String, sizeBytes: Long) {
        jdbc.update(
            "insert into object_gc(object_key,size_bytes,not_before) values (?,?,now()+interval '$STAGING_HOURS hours') on conflict do nothing",
            key,
            sizeBytes,
        )
    }

    /** Блокирует резерв [key] и сообщает, жив ли он (не просрочен и не взят GC в удаление). */
    fun lockLiveStaging(key: String): Boolean = jdbc.queryForList(
        "select object_key from object_gc where object_key=? and not deleting and not_before>now() for update",
        key,
    ).isNotEmpty()

    /** Объект опубликован (или больше не нужен GC): снимает запись. */
    fun remove(key: String) {
        jdbc.update("delete from object_gc where object_key=?", key)
    }

    /** Отдаёт [key] на немедленное удаление. */
    fun scheduleDeletion(key: String, sizeBytes: Long) {
        jdbc.update("insert into object_gc(object_key,size_bytes,not_before) values (?,?,now()) on conflict do nothing", key, sizeBytes)
    }

    /** Артефакты задачи и её проверок уходят в GC (если их не держит блоб). */
    fun scheduleArtifactsOfJobTree(jobId: String) {
        jdbc.update(
            """insert into object_gc(object_key,size_bytes,not_before)
               select a.object_key,a.size_bytes,now() from artifacts a
               where (a.job_id=? or a.job_id in (select c.id from queue_jobs c where c.parent_job_id=?))
               and not exists(select 1 from blobs b where b.object_key=a.object_key) on conflict do nothing""",
            jobId,
            jobId,
        )
    }

    /** Недозагруженные файлы просроченных загрузок. */
    fun scheduleExpiredUploadInputs() {
        jdbc.update(
            """insert into object_gc(object_key,size_bytes,not_before) select 'inputs/'||id,expected_bytes,now() from uploads
               where expires_at<now() and state in ${UploadState.IN_PROGRESS.sqlList()} on conflict do nothing""",
        )
    }

    /** Диагностические артефакты старше [days] дней. */
    fun scheduleOldDebugArtifacts(days: Int) {
        jdbc.update(
            """insert into object_gc(object_key,size_bytes,not_before) select object_key,size_bytes,now() from artifacts
               where debug=true and created_at<now()-interval '$days days' on conflict do nothing""",
        )
    }

    /** Берёт в удаление до [limit] ключей, на которые больше никто не ссылается; возвращает их. */
    @Transactional
    fun claimDeletions(limit: Int): List<String> = jdbc.query(
        """update object_gc set deleting=true where object_key in (
           select g.object_key from object_gc g where not_before<now()
           and not exists(select 1 from artifacts a where a.object_key=g.object_key)
           and not exists(select 1 from blobs b where b.object_key=g.object_key)
           order by g.object_key for update skip locked limit $limit) returning object_key""",
    ) { rs, _ -> rs.getString(1) }

    companion object {
        /** Сколько живёт резерв под заливку объекта. */
        const val STAGING_HOURS = 2
    }
}
