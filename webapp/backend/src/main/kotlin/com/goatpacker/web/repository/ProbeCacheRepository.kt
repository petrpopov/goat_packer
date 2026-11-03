package com.goatpacker.web.repository

import org.springframework.context.annotation.Profile
import org.springframework.jdbc.core.JdbcTemplate
import org.springframework.stereotype.Repository

/**
 * Записи кэша проб (`probe_cache_entries`). Вытеснение и срок удаляют строки и в той же команде ставят их объекты
 * в очередь удаления [ObjectGcRepository], поэтому объект без строки не теряется, а строка без объекта не остаётся.
 */
@Repository
@Profile("server")
class ProbeCacheRepository(private val jdbc: JdbcTemplate) {
    /** Записи модели [group] владельца [owner], свежие первыми. */
    fun list(owner: String, group: String, limit: Int): List<ProbeCacheEntry> = jdbc.query(
        "select name,object_key,size_bytes from probe_cache_entries where owner_id=? and group_key=? order by last_used_at desc,name limit ?",
        { rs, _ -> ProbeCacheEntry(rs.getString(1), rs.getString(2), rs.getLong(3)) },
        owner,
        group,
        limit,
    )

    fun names(owner: String, group: String): Set<String> = jdbc.queryForList(
        "select name from probe_cache_entries where owner_id=? and group_key=?",
        String::class.java,
        owner,
        group,
    ).toSet()

    fun touch(owner: String, group: String, names: Collection<String>) {
        if (names.isEmpty()) return
        jdbc.update(
            "update probe_cache_entries set last_used_at=now() where owner_id=? and group_key=? and name = any(?)",
            owner,
            group,
            names.toTypedArray(),
        )
    }

    /** false — запись с таким именем уже есть (параллельная джоба успела раньше). */
    fun insert(owner: String, group: String, entry: ProbeCacheEntry): Boolean = jdbc.update(
        "insert into probe_cache_entries(owner_id,group_key,name,object_key,size_bytes,created_at,last_used_at) values (?,?,?,?,?,now(),now()) on conflict do nothing",
        owner,
        group,
        entry.name,
        entry.objectKey,
        entry.sizeBytes,
    ) == 1

    /** Оставляет в модели [group] не больше [maxEntries] свежих записей. */
    fun evictGroup(owner: String, group: String, maxEntries: Int): Int = evict(
        """select owner_id,group_key,name from (select owner_id,group_key,name,
           row_number() over (order by last_used_at desc,name) as n from probe_cache_entries where owner_id=? and group_key=?) t where n>?""",
        owner,
        group,
        maxEntries,
    )

    /** Оставляет у владельца свежие записи общим размером не больше [maxBytes]. */
    fun evictOwner(owner: String, maxBytes: Long): Int = evict(
        """select owner_id,group_key,name from (select owner_id,group_key,name,
           sum(size_bytes) over (order by last_used_at desc,group_key,name) as running from probe_cache_entries where owner_id=?) t where running>?""",
        owner,
        maxBytes,
    )

    /** Записи, не использовавшиеся [days] дней. */
    fun evictUnusedFor(days: Int): Int = evict(
        "select owner_id,group_key,name from probe_cache_entries where last_used_at<now()-make_interval(days => ?)",
        days,
    )

    private fun evict(victims: String, vararg args: Any): Int = jdbc.update(
        """with victims as ($victims),
           gone as (delete from probe_cache_entries e using victims v
             where e.owner_id=v.owner_id and e.group_key=v.group_key and e.name=v.name returning e.object_key,e.size_bytes)
           insert into object_gc(object_key,size_bytes,not_before) select object_key,size_bytes,now() from gone on conflict do nothing""",
        *args,
    )
}

data class ProbeCacheEntry(val name: String, val objectKey: String, val sizeBytes: Long)
