package com.goatpacker.web.controller

import com.goatpacker.web.dto.JobCreatedDto
import com.goatpacker.web.dto.ScheduleJobForm
import com.goatpacker.web.service.ScheduleService
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
@RequestMapping("/api/schedule")
class ScheduleController(
    private val scheduleService: ScheduleService,
) {
    @PostMapping("/jobs", consumes = [MediaType.MULTIPART_FORM_DATA_VALUE])
    fun createJob(@Valid @ModelAttribute form: ScheduleJobForm): ResponseEntity<JobCreatedDto> =
        ResponseEntity.status(HttpStatus.ACCEPTED).body(scheduleService.create(form))
}
