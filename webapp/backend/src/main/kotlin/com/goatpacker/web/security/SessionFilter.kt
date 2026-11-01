package com.goatpacker.web.security

import com.goatpacker.web.exception.ApiException
import com.goatpacker.web.service.server.SessionService
import jakarta.servlet.FilterChain
import jakarta.servlet.http.HttpServletRequest
import jakarta.servlet.http.HttpServletResponse
import org.springframework.security.authentication.UsernamePasswordAuthenticationToken
import org.springframework.security.core.authority.SimpleGrantedAuthority
import org.springframework.security.core.context.SecurityContextHolder
import org.springframework.web.filter.OncePerRequestFilter

/**
 * Достаёт сессию из cookie и кладёт [Actor] в SecurityContext. Не бин: подключается только в цепочку
 * безопасности сервера. `logout` и `csrf` идут без сессии, чтобы их можно было вызвать с протухшей cookie.
 */
class SessionFilter(
    private val sessions: SessionService,
    private val secureCookies: Boolean,
) : OncePerRequestFilter() {
    override fun doFilterInternal(req: HttpServletRequest, res: HttpServletResponse, chain: FilterChain) {
        try {
            if (req.requestURI !in SESSIONLESS_PATHS) authenticate(req, res)
            chain.doFilter(req, res)
        } catch (ex: ApiException) {
            writeError(res, ex.status.value(), ex.code, ex.message.orEmpty())
        }
    }

    private fun authenticate(req: HttpServletRequest, res: HttpServletResponse) {
        val token = sessionToken(req)
        val actor = sessions.authenticate(token) {
            res.addHeader("Set-Cookie", sessionCookie(token!!, secureCookies))
        } ?: return
        SecurityContextHolder.getContext().authentication = UsernamePasswordAuthenticationToken(
            actor,
            null,
            listOf(SimpleGrantedAuthority("ROLE_${actor.role}")),
        )
    }

    private companion object {
        val SESSIONLESS_PATHS = setOf("/api/v2/auth/logout", "/api/v2/auth/csrf")
    }
}

/** Токен сессии из cookie запроса. */
fun sessionToken(req: HttpServletRequest): String? = req.cookies?.find { it.name == SESSION_COOKIE_NAME }?.value
