package com.goatpacker.web.controller

import com.goatpacker.web.dto.JobCreatedDto
import com.goatpacker.web.dto.PackJobForm
import com.goatpacker.web.service.PackService
import jakarta.validation.Valid
import org.springframework.http.HttpStatus
import org.springframework.http.MediaType
import org.springframework.http.ResponseEntity
import org.springframework.web.bind.annotation.ModelAttribute
import org.springframework.web.bind.annotation.PostMapping
import org.springframework.web.bind.annotation.RequestMapping
import org.springframework.web.bind.annotation.RestController

@RestController
@org.springframework.context.annotation.Profile("!server")
@RequestMapping("/api/pack")
class PackController(
    private val packService: PackService,
) {
    @PostMapping("/jobs", consumes = [MediaType.MULTIPART_FORM_DATA_VALUE])
    fun createJob(@Valid @ModelAttribute form: PackJobForm): ResponseEntity<JobCreatedDto> =
        ResponseEntity.status(HttpStatus.ACCEPTED).body(packService.create(form))
}
