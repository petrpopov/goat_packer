package com.goatpacker.web.service.server

import com.goatpacker.web.exception.ErrorMessages
import com.goatpacker.web.exception.badRequest
import java.util.Locale

/** Правила имён, паролей и причин, общие для аккаунтов и администрирования. */
object AccountPolicy {
    /** Потолок длины пароля при входе: bcrypt не должен хешировать произвольно длинные строки. */
    const val MAX_LOGIN_PASSWORD_LENGTH = 128

    private const val MIN_PASSWORD_LENGTH = 12
    private const val MAX_PASSWORD_BYTES = 72 // предел bcrypt: остальное он молча отбросил бы
    private val LOGIN_PATTERN = Regex("[a-z0-9][a-z0-9_.@+\\-]{2,119}")

    val REASON_LENGTH = 3..500

    fun normalizeLogin(login: String): String {
        val value = login.trim().lowercase(Locale.ROOT)
        if (!LOGIN_PATTERN.matches(value)) badRequest("Имя: 3–120 латинских букв, цифр или символов _.@+-")
        return value
    }

    /** Для входа имя только приводится к виду хранения: проверка формата нужна лишь при регистрации. */
    fun loginKey(login: String): String = login.trim().lowercase(Locale.ROOT)

    fun requireValidPassword(password: String) {
        if (password.length < MIN_PASSWORD_LENGTH || password.toByteArray().size > MAX_PASSWORD_BYTES) {
            badRequest("Пароль: минимум $MIN_PASSWORD_LENGTH символов, максимум $MAX_PASSWORD_BYTES байта")
        }
    }

    /** Причина действия администратора: возвращает её без пробелов по краям. */
    fun requireReason(reason: String): String {
        val trimmed = reason.trim()
        if (trimmed.length !in REASON_LENGTH) badRequest(ErrorMessages.REASON_REQUIRED)
        return trimmed
    }
}
