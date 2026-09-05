package com.goatpacker.web.controller

import com.goatpacker.web.dto.ProfileDto
import com.goatpacker.web.service.ProfileService
import org.springframework.web.bind.annotation.GetMapping
import org.springframework.web.bind.annotation.RequestMapping
import org.springframework.web.bind.annotation.RestController

@RestController
@RequestMapping("/api/profiles")
class ProfilesController(
    private val profileService: ProfileService,
) {
    @GetMapping
    fun list(): List<ProfileDto> = profileService.list()
}
