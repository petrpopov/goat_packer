package com.goatpacker.web.security

import com.fasterxml.jackson.databind.ObjectMapper
import jakarta.servlet.http.HttpServletResponse
import org.springframework.http.ResponseCookie

/** Срок сессии: столько живут и строка `auth_sessions` (с последнего продления), и cookie. */
const val SESSION_SECONDS = 7L * 86400

const val SESSION_COOKIE_NAME = "GP_SESSION"

fun sessionCookie(token: String, secure: Boolean, maxAge: Long = SESSION_SECONDS): String =
    ResponseCookie.from(SESSION_COOKIE_NAME, token)
        .httpOnly(true)
        .secure(secure)
        .sameSite("Strict")
        .path("/")
        .maxAge(maxAge)
        .build()
        .toString()

private val errorJson = ObjectMapper()

/**
 * Ответ об ошибке вне контроллеров (фильтр, точки входа Spring Security) — всегда UTF-8: без явной кодировки
 * сервлет пишет ISO-8859-1, и браузер показывал вместо русского сообщения «????? ???? ??? ???????? ??????».
 */
fun writeError(res: HttpServletResponse, status: Int, code: String, message: String) {
    res.status = status
    res.contentType = "application/json"
    res.characterEncoding = "UTF-8"
    res.writer.write(errorJson.writeValueAsString(mapOf("code" to code, "message" to message)))
}
