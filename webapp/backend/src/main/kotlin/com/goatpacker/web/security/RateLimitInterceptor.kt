package com.goatpacker.web.security

import com.goatpacker.web.service.server.RateLimiter
import com.goatpacker.web.service.server.RateRule
import com.goatpacker.web.util.SECONDS_PER_HOUR
import jakarta.servlet.http.HttpServletRequest
import jakarta.servlet.http.HttpServletResponse
import org.springframework.context.annotation.Profile
import org.springframework.stereotype.Component
import org.springframework.web.servlet.HandlerInterceptor

/** Один счётчик маршрута: общий на весь сервис ([perClient] = false) или отдельный для каждого адреса клиента. */
private data class Throttle(val name: String, val rule: RateRule, val perClient: Boolean = false) {
    fun key(clientAddress: String): String = if (perClient) "$name:$clientAddress" else name
}

/** Маршрут, который закрывает защита от перебора и массового создания. */
private data class ThrottledRoute(val method: String, val path: Regex, val throttles: List<Throttle>)

/**
 * Ограничение частоты запросов на входах, доступных без проверки владельца (гость, вход, регистрация,
 * сброс пароля) и на создании задач и загрузок. Работает до контроллера и считает обращение в своей
 * транзакции: неудачные попытки (неверный пароль, отказ валидации) не откатывают счётчик.
 */
@Component
@Profile("server")
class RateLimitInterceptor(private val limiter: RateLimiter) : HandlerInterceptor {
    override fun preHandle(request: HttpServletRequest, response: HttpServletResponse, handler: Any): Boolean {
        val route = ROUTES.firstOrNull { it.method == request.method && it.path.matches(request.requestURI) }
        route?.throttles?.forEach { limiter.check(it.key(request.remoteAddr), it.rule) }
        return true
    }

    private companion object {
        const val POST = "POST"

        val ROUTES = listOf(
            ThrottledRoute(
                POST,
                Regex("/api/v2/auth/guest"),
                listOf(
                    Throttle("guest-global", RateRule(60)),
                    Throttle("guest", RateRule(10, SECONDS_PER_HOUR), perClient = true),
                ),
            ),
            ThrottledRoute(
                POST,
                Regex("/api/v2/auth/register"),
                listOf(
                    Throttle("signup-global", RateRule(10)),
                    Throttle("signup", RateRule(5, SECONDS_PER_HOUR), perClient = true),
                ),
            ),
            ThrottledRoute(
                POST,
                Regex("/api/v2/auth/login"),
                listOf(
                    Throttle("login-global", RateRule(30)),
                    Throttle("login", RateRule(10), perClient = true),
                ),
            ),
            ThrottledRoute(POST, Regex("/api/v2/auth/reset"), listOf(Throttle("reset", RateRule(5), perClient = true))),
            ThrottledRoute(
                POST,
                Regex("/api/v2/jobs(/[^/]+/check)?"),
                listOf(Throttle("jobs-ip", RateRule(30, SECONDS_PER_HOUR), perClient = true)),
            ),
            ThrottledRoute(
                POST,
                Regex("/api/v2/uploads"),
                listOf(
                    Throttle("uploads", RateRule(10, SECONDS_PER_HOUR), perClient = true),
                    Throttle("upload-global", RateRule(100, SECONDS_PER_HOUR)),
                ),
            ),
        )
    }
}
