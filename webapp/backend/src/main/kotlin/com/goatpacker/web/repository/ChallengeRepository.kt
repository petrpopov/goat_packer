package com.goatpacker.web.repository

import org.springframework.context.annotation.Profile
import org.springframework.jdbc.core.JdbcTemplate
import org.springframework.stereotype.Repository
import java.sql.Timestamp
import java.time.Instant

/** Одноразовая задача на проверку «не бот» (`bot_challenges`). */
@Repository
@Profile("server")
class ChallengeRepository(private val jdbc: JdbcTemplate) {
    fun insert(id: String, ownerId: String, salt: String, difficulty: Int, expiresAt: Instant) {
        jdbc.update(
            "insert into bot_challenges(id,owner_id,salt,difficulty,expires_at) values (?,?,?,?,?)",
            id,
            ownerId,
            salt,
            difficulty,
            Timestamp.from(expiresAt),
        )
    }

    /** Блокирует строку задачи на время проверки решения; null — такой задачи нет. */
    fun lockById(id: String): StoredChallenge? = jdbc.query(
        "select owner_id,salt,difficulty,expires_at,used from bot_challenges where id=? for update",
        { rs, _ ->
            StoredChallenge(
                ownerId = rs.getString("owner_id"),
                salt = rs.getString("salt"),
                difficulty = rs.getInt("difficulty"),
                expiresAt = rs.getTimestamp("expires_at").toInstant(),
                used = rs.getBoolean("used"),
            )
        },
        id,
    ).firstOrNull()

    fun markUsed(id: String) {
        jdbc.update("update bot_challenges set used=true where id=?", id)
    }
}

data class StoredChallenge(
    val ownerId: String,
    val salt: String,
    val difficulty: Int,
    val expiresAt: Instant,
    val used: Boolean,
)
