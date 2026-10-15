package com.goatpacker.web.server

import org.springframework.context.annotation.Profile
import org.springframework.jdbc.core.JdbcTemplate
import org.springframework.scheduling.annotation.Scheduled
import org.springframework.stereotype.Service
import org.springframework.transaction.annotation.Transactional

/** Reference-aware deletion: enqueue immutable keys in PG, delete from S3 after commit, retry failures. */
@Service @Profile("server")
class RetentionTransactions(private val jdbc:JdbcTemplate,private val config:PlatformRepository) {
    @Transactional fun claimDeletions():List<String> = jdbc.query("""update object_gc set deleting=true where object_key in (
        select g.object_key from object_gc g where not_before<now()
        and not exists(select 1 from artifacts a where a.object_key=g.object_key)
        and not exists(select 1 from blobs b where b.object_key=g.object_key)
        order by g.object_key for update skip locked limit 100) returning object_key""",{r,_->r.getString(1)})
    @Transactional fun expire() {
        config.locked()
        val expired=jdbc.query("select id from queue_jobs where expires_at<now() and status not in ('QUEUED','RUNNING','FINALIZING','LOST','RETRY_WAIT') limit 100",{r,_->r.getString(1)})
        for(id in expired) {
            jdbc.update("""insert into object_gc(object_key,size_bytes,not_before)
                select a.object_key,a.size_bytes,now() from artifacts a where job_id=? and not exists(select 1 from blobs b where b.object_key=a.object_key)
                on conflict do nothing""",id)
            jdbc.update("delete from queue_jobs where id=?",id)
        }
        jdbc.update("""insert into object_gc(object_key,size_bytes,not_before) select 'inputs/'||id,expected_bytes,now() from uploads
            where expires_at<now() and state in ('UPLOADING','RECEIVING') on conflict do nothing""")
        jdbc.update("delete from uploads where expires_at<now()")
        // Registered diagnostic files expire separately, without expiring their useful 3MF/CSV/XLSX.
        jdbc.update("""insert into object_gc(object_key,size_bytes,not_before) select object_key,size_bytes,now() from artifacts
            where debug=true and created_at<now()-interval '7 days' on conflict do nothing""")
        jdbc.update("delete from artifacts where debug=true and created_at<now()-interval '7 days'")
        val unused=jdbc.query("""select id,object_key,size_bytes from blobs b where not exists(select 1 from uploads u where u.blob_id=b.id)
            and not exists(select 1 from queue_jobs j where j.input_blob_id=b.id) and not exists(select 1 from artifacts a where a.object_key=b.object_key) limit 100""",{r,_->Triple(r.getString(1),r.getString(2),r.getLong(3))})
        for((id,key,size) in unused) {
            jdbc.update("insert into object_gc(object_key,size_bytes,not_before) values (?,?,now()) on conflict do nothing",key,size)
            jdbc.update("delete from blobs where id=?",id)
        }
        jdbc.update("delete from auth_sessions where expires_at<now()")
        jdbc.update("delete from password_resets where expires_at<now()")
        jdbc.update("delete from read_grants where expires_at<now()")
        jdbc.update("""delete from principals p where p.kind='GUEST' and not p.blocked and p.last_seen_at<now()-interval '7 days'
            and not exists(select 1 from auth_sessions s where s.owner_id=p.id)
            and not exists(select 1 from uploads u where u.owner_id=p.id)
            and not exists(select 1 from queue_jobs j where j.owner_id=p.id)
            and not exists(select 1 from read_grants g where g.owner_id=p.id)
            and not exists(select 1 from bot_challenges c where c.owner_id=p.id)""")
        jdbc.update("delete from bot_challenges where expires_at<now()")
        jdbc.update("delete from compute_budgets where day<current_date-7")
        jdbc.update("delete from request_limits where window_start<now()-interval '2 days'")
    }
}
@Service @Profile("server")
class Retention(private val tx:RetentionTransactions,private val jdbc:JdbcTemplate,private val store:ObjectStore,private val queue:DurableQueue) {
    @Scheduled(fixedDelay=60000,initialDelay=60000) fun collect() {
        queue.recover();tx.expire()
        tx.claimDeletions().forEach { key->
            runCatching {store.delete(key);jdbc.update("delete from object_gc where object_key=?",key)}
        }
    }
}
