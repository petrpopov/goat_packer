package com.goatpacker.web.config

import com.goatpacker.web.entity.Role
import com.goatpacker.web.exception.ErrorCode
import com.goatpacker.web.security.SessionFilter
import com.goatpacker.web.security.writeError
import com.goatpacker.web.service.server.SessionService
import org.springframework.boot.autoconfigure.condition.ConditionalOnWebApplication
import org.springframework.context.annotation.Bean
import org.springframework.context.annotation.Configuration
import org.springframework.context.annotation.Profile
import org.springframework.http.HttpStatus
import org.springframework.security.config.annotation.web.builders.HttpSecurity
import org.springframework.security.web.SecurityFilterChain
import org.springframework.security.web.authentication.UsernamePasswordAuthenticationFilter
import org.springframework.security.web.context.NullSecurityContextRepository
import org.springframework.security.web.csrf.CookieCsrfTokenRepository
import org.springframework.security.web.csrf.CsrfTokenRequestAttributeHandler

/** Безопасность серверного режима: сессия в cookie, CSRF-токен, `/api/v2/admin` только для роли ADMIN. */
@Configuration
@Profile("server")
@ConditionalOnWebApplication
class ServerSecurityConfig {
    @Bean
    fun serverChain(http: HttpSecurity, sessions: SessionService, props: PlatformProperties): SecurityFilterChain {
        val csrf = CookieCsrfTokenRepository.withHttpOnlyFalse().apply {
            setCookieCustomizer { it.secure(props.secureCookies).sameSite("Strict").path("/") }
        }
        return http.cors { }
            .sessionManagement { it.disable() }
            .securityContext { it.securityContextRepository(NullSecurityContextRepository()) }
            .requestCache { it.disable() }
            .csrf { it.csrfTokenRepository(csrf).csrfTokenRequestHandler(CsrfTokenRequestAttributeHandler()) }
            .addFilterBefore(SessionFilter(sessions, props.secureCookies), UsernamePasswordAuthenticationFilter::class.java)
            .authorizeHttpRequests {
                it.requestMatchers(*PUBLIC_PATHS).permitAll()
                it.requestMatchers("/api/v2/admin/**").hasRole(Role.ADMIN.name)
                it.requestMatchers("/api/v2/**").authenticated()
                it.anyRequest().denyAll()
            }
            .exceptionHandling {
                it.authenticationEntryPoint { _, res, _ ->
                    writeError(res, HttpStatus.UNAUTHORIZED.value(), ErrorCode.UNAUTHORIZED, "Сессия истекла — обновите страницу")
                }
                it.accessDeniedHandler { _, res, _ ->
                    writeError(res, HttpStatus.FORBIDDEN.value(), ErrorCode.FORBIDDEN, "Недостаточно прав или неверный CSRF-токен")
                }
            }
            .build()
    }

    private companion object {
        /** Доступны без сессии: проверка живости и входы в сервис. */
        val PUBLIC_PATHS = arrayOf(
            "/api/v2/health",
            "/api/v2/auth/csrf",
            "/api/v2/auth/guest",
            "/api/v2/auth/login",
            "/api/v2/auth/register",
            "/api/v2/auth/reset",
            "/api/v2/auth/logout",
        )
    }
}
