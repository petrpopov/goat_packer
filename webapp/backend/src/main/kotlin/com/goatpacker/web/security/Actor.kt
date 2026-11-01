package com.goatpacker.web.security

import com.goatpacker.web.entity.PrincipalKind
import com.goatpacker.web.entity.Role
import com.goatpacker.web.exception.ApiException
import com.goatpacker.web.exception.ErrorCode
import org.springframework.http.HttpStatus
import org.springframework.security.core.context.SecurityContextHolder

/** Аутентифицированный владелец запроса; кладётся в SecurityContext фильтром сессии. */
data class Actor(val id: String, val role: Role, val kind: PrincipalKind, val login: String?)

/** Актор текущего запроса или null (гость ещё не заведён, вход не выполнен). */
fun currentActorOrNull(): Actor? = SecurityContextHolder.getContext().authentication?.principal as? Actor

fun currentActor(): Actor = currentActorOrNull()
    ?: throw ApiException(HttpStatus.UNAUTHORIZED, ErrorCode.UNAUTHORIZED, "Нужна гостевая сессия или вход")
