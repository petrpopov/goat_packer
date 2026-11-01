package com.goatpacker.web.repository

import com.goatpacker.web.entity.UploadState
import org.springframework.context.annotation.Profile
import org.springframework.jdbc.core.JdbcTemplate
import org.springframework.stereotype.Repository
import java.time.LocalDate

/** Режим обслуживания (строка `operational_state`): новые задачи и загрузки не принимаются. */
@Repository
@Profile("server")
class OperationalStateRepository(private val jdbc: JdbcTemplate) {
    fun isMaintenance(): Boolean =
        jdbc.queryForObject("select maintenance from operational_state where id=1", Boolean::class.java) == true

    /** Проверка связи с БД. */
    fun ping() {
        jdbc.queryForObject("select 1", Int::class.java)
    }
}

/** Счётчики окон частоты запросов (`request_limits`). */
@Repository
@Profile("server")
class RequestLimitRepository(private val jdbc: JdbcTemplate) {
    /** Засчитывает обращение по [key] и возвращает число обращений в текущем окне длиной [windowSeconds]. */
    fun recordHit(key: String, windowSeconds: Long): Int = jdbc.queryForObject(
        """insert into request_limits(key,window_start,used) values (?,now(),1)
           on conflict(key) do update set
           used=case when request_limits.window_start < now()- (? * interval '1 second') then 1 else request_limits.used+1 end,
           window_start=case when request_limits.window_start < now()- (? * interval '1 second') then now() else request_limits.window_start end
           returning used""",
        Int::class.java,
        key,
        windowSeconds,
        windowSeconds,
    )!!
}

/** Суточный вычислительный бюджет (`compute_budgets`): владелец и сводная строка [GLOBAL_OWNER]. */
@Repository
@Profile("server")
class ComputeBudgetRepository(private val jdbc: JdbcTemplate) {
    /** Занято на день: потрачено плюс зарезервировано под идущие и ожидающие задачи. */
    fun committedSeconds(owner: String, day: LocalDate): Long {
        jdbc.update("insert into compute_budgets(owner_id,day) values (?,?) on conflict do nothing", owner, day)
        return jdbc.queryForObject(
            "select used_seconds+reserved_seconds from compute_budgets where owner_id=? and day=?",
            Long::class.java,
            owner,
            day,
        )!!
    }

    fun reserve(owner: String, day: LocalDate, seconds: Long) {
        jdbc.update("update compute_budgets set reserved_seconds=reserved_seconds+? where owner_id=? and day=?", seconds, owner, day)
    }

    /** Часть резерва фактически потрачена: переходит из reserved в used. */
    fun charge(owner: String, day: LocalDate, seconds: Long) {
        jdbc.update(
            "update compute_budgets set reserved_seconds=greatest(0,reserved_seconds-?),used_seconds=used_seconds+? where owner_id=? and day=?",
            seconds,
            seconds,
            owner,
            day,
        )
    }

    /** Возвращает неиспользованный резерв. */
    fun release(owner: String, day: LocalDate, seconds: Long) {
        jdbc.update("update compute_budgets set reserved_seconds=greatest(0,reserved_seconds-?) where owner_id=? and day=?", seconds, owner, day)
    }

    companion object {
        /** Владелец сводной строки бюджета на весь сервис. */
        const val GLOBAL_OWNER = "GLOBAL"
    }
}

/** Занятое место в хранилище: личное (для квоты владельца) и общее (потолок диска). */
@Repository
@Profile("server")
class StorageUsageRepository(private val jdbc: JdbcTemplate) {
    fun ownerBytes(owner: String): Long = jdbc.queryForObject(
        """select
           coalesce((select sum(expected_bytes) from uploads where owner_id=?),0)+
           coalesce((select sum(reserved_bytes) from queue_jobs where owner_id=?),0)+
           coalesce((select sum(a.size_bytes) from artifacts a join queue_jobs j on j.id=a.job_id where j.owner_id=?),0)""",
        Long::class.java,
        owner,
        owner,
        owner,
    )!!

    fun totalBytes(): Long = jdbc.queryForObject(
        """select coalesce((select sum(size_bytes) from blobs),0)+
           coalesce((select sum(size_bytes) from artifacts),0)+
           coalesce((select sum(expected_bytes) from uploads where state in ${UploadState.IN_PROGRESS.sqlList()}),0)+
           coalesce((select sum(reserved_bytes) from queue_jobs),0)+
           coalesce((select sum(size_bytes) from object_gc),0)""",
        Long::class.java,
    )!!
}
