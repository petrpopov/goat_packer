package com.goatpacker.web.controller.local

import com.goatpacker.web.dto.JobCreatedDto
import com.goatpacker.web.dto.PackJobForm
import com.goatpacker.web.service.local.PackCheckService
import com.goatpacker.web.service.local.PackService
import jakarta.validation.Valid
import org.springframework.context.annotation.Profile
import org.springframework.http.HttpStatus
import org.springframework.http.MediaType
import org.springframework.http.ResponseEntity
import org.springframework.web.bind.annotation.ModelAttribute
import org.springframework.web.bind.annotation.PathVariable
import org.springframework.web.bind.annotation.PostMapping
import org.springframework.web.bind.annotation.RequestMapping
import org.springframework.web.bind.annotation.RestController

@RestController
@Profile("!server")
@RequestMapping("/api/pack")
class PackController(
    private val packService: PackService,
    private val checkService: PackCheckService,
) {
    @PostMapping("/jobs", consumes = [MediaType.MULTIPART_FORM_DATA_VALUE])
    fun createJob(@Valid @ModelAttribute form: PackJobForm): ResponseEntity<JobCreatedDto> =
        ResponseEntity.status(HttpStatus.ACCEPTED).body(packService.create(form))

    /** «Нарезать стол»: полная нарезка упаковки, собранной без неё. */
    @PostMapping("/jobs/{id}/finish")
    fun finishJob(@PathVariable id: String): ResponseEntity<JobCreatedDto> =
        ResponseEntity.status(HttpStatus.ACCEPTED).body(checkService.start(id))

    /** Отмена идущей проверки нарезкой; упаковка остаётся как была. */
    @PostMapping("/jobs/{id}/check/cancel")
    fun cancelCheck(@PathVariable id: String): ResponseEntity<Void> {
        checkService.cancel(id)
        return ResponseEntity.noContent().build()
    }
}
