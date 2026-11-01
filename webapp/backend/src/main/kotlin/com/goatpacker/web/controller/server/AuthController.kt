package com.goatpacker.web.controller.server

import com.goatpacker.web.config.PlatformProperties
import com.goatpacker.web.dto.CredentialsRequest
import com.goatpacker.web.dto.CsrfTokenDto
import com.goatpacker.web.dto.PrincipalDto
import com.goatpacker.web.dto.ResetPasswordRequest
import com.goatpacker.web.security.Actor
import com.goatpacker.web.security.sessionCookie
import com.goatpacker.web.security.sessionToken
import com.goatpacker.web.service.server.AccountService
import com.goatpacker.web.service.server.PasswordResetService
import com.goatpacker.web.service.server.SessionService
import com.goatpacker.web.service.server.SignedIn
import jakarta.servlet.http.HttpServletRequest
import jakarta.servlet.http.HttpServletResponse
import org.springframework.context.annotation.Profile
import org.springframework.http.HttpHeaders
import org.springframework.security.core.annotation.AuthenticationPrincipal
import org.springframework.security.web.csrf.CsrfToken
import org.springframework.web.bind.annotation.GetMapping
import org.springframework.web.bind.annotation.PostMapping
import org.springframework.web.bind.annotation.RequestBody
import org.springframework.web.bind.annotation.RequestMapping
import org.springframework.web.bind.annotation.RestController

/** Гость, вход, регистрация, выход и сброс пароля. Частоту обращений ограничивает RateLimitInterceptor. */
@RestController
@Profile("server")
@RequestMapping("/api/v2/auth")
class AuthController(
    private val accounts: AccountService,
    private val sessions: SessionService,
    private val passwordResets: PasswordResetService,
    private val props: PlatformProperties,
) {
    @GetMapping("/csrf")
    fun csrf(token: CsrfToken) = CsrfTokenDto(token.token)

    @GetMapping("/me")
    fun me(@AuthenticationPrincipal actor: Actor): PrincipalDto = accounts.currentUser(actor)

    @PostMapping("/guest")
    fun guest(@AuthenticationPrincipal actor: Actor?, res: HttpServletResponse): PrincipalDto =
        if (actor != null) accounts.currentUser(actor) else respond(accounts.createGuest(), res)

    @PostMapping("/register")
    fun register(
        @RequestBody form: CredentialsRequest,
        @AuthenticationPrincipal actor: Actor?,
        res: HttpServletResponse,
    ): PrincipalDto = respond(accounts.register(form.login, form.password, actor), res)

    @PostMapping("/login")
    fun login(@RequestBody form: CredentialsRequest, res: HttpServletResponse): PrincipalDto =
        respond(accounts.login(form.login, form.password), res)

    @PostMapping("/logout")
    fun logout(req: HttpServletRequest, res: HttpServletResponse) {
        sessions.close(sessionToken(req))
        res.addHeader(HttpHeaders.SET_COOKIE, sessionCookie("", props.secureCookies, maxAge = 0))
    }

    @PostMapping("/reset")
    fun reset(@RequestBody form: ResetPasswordRequest) = passwordResets.consume(form.token, form.password)

    private fun respond(signedIn: SignedIn, res: HttpServletResponse): PrincipalDto {
        res.addHeader(HttpHeaders.SET_COOKIE, sessionCookie(signedIn.sessionToken, props.secureCookies))
        return signedIn.user
    }
}
