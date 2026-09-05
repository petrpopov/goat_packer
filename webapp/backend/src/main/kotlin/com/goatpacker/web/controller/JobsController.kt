package com.goatpacker.web.controller

import com.goatpacker.web.dto.JobStatusDto
import com.goatpacker.web.service.JobService
import org.springframework.core.io.FileSystemResource
import org.springframework.http.ResponseEntity
import org.springframework.web.bind.annotation.DeleteMapping
import org.springframework.web.bind.annotation.GetMapping
import org.springframework.web.bind.annotation.PathVariable
import org.springframework.web.bind.annotation.RequestMapping
import org.springframework.web.bind.annotation.RestController

@RestController
@RequestMapping("/api/jobs")
class JobsController(
    private val jobService: JobService,
) {
    @GetMapping("/{id}")
    fun status(@PathVariable id: String): JobStatusDto = jobService.status(id)

    @GetMapping("/{id}/artifacts/{name}")
    fun artifact(@PathVariable id: String, @PathVariable name: String): ResponseEntity<FileSystemResource> =
        jobService.artifact(id, name)

    @DeleteMapping("/{id}")
    fun delete(@PathVariable id: String): ResponseEntity<Void> = jobService.delete(id)
}
