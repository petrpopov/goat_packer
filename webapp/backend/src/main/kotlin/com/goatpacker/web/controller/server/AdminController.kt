package com.goatpacker.web.controller.server

import com.goatpacker.web.dto.AuditEventDto
import com.goatpacker.web.dto.BlockRequest
import com.goatpacker.web.dto.PasswordResetIssuedDto
import com.goatpacker.web.dto.ReasonRequest
import com.goatpacker.web.dto.SystemSettingsRequest
import com.goatpacker.web.dto.SystemStateDto
import com.goatpacker.web.dto.UserPageDto
import com.goatpacker.web.security.Actor
import com.goatpacker.web.service.server.AdminService
import com.goatpacker.web.service.server.PasswordResetService
import org.springframework.context.annotation.Profile
import org.springframework.security.core.annotation.AuthenticationPrincipal
import org.springframework.web.bind.annotation.GetMapping
import org.springframework.web.bind.annotation.PathVariable
import org.springframework.web.bind.annotation.PostMapping
import org.springframework.web.bind.annotation.RequestBody
import org.springframework.web.bind.annotation.RequestMapping
import org.springframework.web.bind.annotation.RequestParam
import org.springframework.web.bind.annotation.RestController

/** Админка: только роль ADMIN (см. цепочку безопасности). */
@RestController
@Profile("server")
@RequestMapping("/api/v2/admin")
class AdminController(
    private val admin: AdminService,
    private val passwordResets: PasswordResetService,
) {
    @GetMapping("/users")
    fun users(
        @RequestParam(defaultValue = "") kind: String,
        @RequestParam(defaultValue = "") query: String,
        @RequestParam(defaultValue = "0") page: Int,
    ): UserPageDto = admin.listUsers(kind, query, page)

    @PostMapping("/users/{id}/block")
    fun block(@PathVariable id: String, @RequestBody request: BlockRequest, @AuthenticationPrincipal actor: Actor) =
        admin.setBlocked(actor.id, id, request.blocked, request.reason)

    @PostMapping("/users/{id}/password-reset")
    fun issuePasswordReset(
        @PathVariable id: String,
        @RequestBody request: ReasonRequest,
        @AuthenticationPrincipal actor: Actor,
    ): PasswordResetIssuedDto = passwordResets.issue(actor.id, id, request.reason)

    @GetMapping("/system")
    fun systemState(): SystemStateDto = admin.systemState()

    @PostMapping("/system")
    fun configure(@RequestBody request: SystemSettingsRequest, @AuthenticationPrincipal actor: Actor) =
        admin.configureParallelism(actor.id, request.maxParallel, request.paused, request.reason)

    @GetMapping("/audit")
    fun audit(@RequestParam(defaultValue = "0") page: Int): List<AuditEventDto> = admin.auditLog(page)
}
