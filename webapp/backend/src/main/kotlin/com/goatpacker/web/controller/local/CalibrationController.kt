package com.goatpacker.web.controller.local

import com.goatpacker.web.dto.CalibrationJobForm
import com.goatpacker.web.dto.JobCreatedDto
import com.goatpacker.web.service.local.CalibrationService
import jakarta.validation.Valid
import org.springframework.context.annotation.Profile
import org.springframework.http.HttpStatus
import org.springframework.http.MediaType
import org.springframework.http.ResponseEntity
import org.springframework.web.bind.annotation.ModelAttribute
import org.springframework.web.bind.annotation.PostMapping
import org.springframework.web.bind.annotation.RequestMapping
import org.springframework.web.bind.annotation.RestController

@RestController
@Profile("!server")
@RequestMapping("/api/calibration")
class CalibrationController(private val calibrationService: CalibrationService) {
    @PostMapping("/jobs", consumes = [MediaType.MULTIPART_FORM_DATA_VALUE])
    fun createJob(@Valid @ModelAttribute form: CalibrationJobForm): ResponseEntity<JobCreatedDto> =
        ResponseEntity.status(HttpStatus.ACCEPTED).body(calibrationService.create(form))
}
