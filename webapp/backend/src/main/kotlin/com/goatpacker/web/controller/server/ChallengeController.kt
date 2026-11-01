package com.goatpacker.web.controller.server

import com.goatpacker.web.dto.ChallengeDto
import com.goatpacker.web.security.Actor
import com.goatpacker.web.service.server.ChallengeService
import org.springframework.context.annotation.Profile
import org.springframework.security.core.annotation.AuthenticationPrincipal
import org.springframework.web.bind.annotation.PostMapping
import org.springframework.web.bind.annotation.RequestMapping
import org.springframework.web.bind.annotation.RestController

@RestController
@Profile("server")
@RequestMapping("/api/v2/challenges")
class ChallengeController(private val challenges: ChallengeService) {
    @PostMapping
    fun issue(@AuthenticationPrincipal actor: Actor): ChallengeDto = challenges.issue(actor.id)
}
