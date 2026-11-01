package com.goatpacker.web.controller.local

import com.goatpacker.web.dto.JobStatusDto
import com.goatpacker.web.service.local.JobService
import com.goatpacker.web.service.local.LocalDownload
import org.springframework.context.annotation.Profile
import org.springframework.core.io.FileSystemResource
import org.springframework.http.ContentDisposition
import org.springframework.http.HttpHeaders
import org.springframework.http.ResponseEntity
import org.springframework.web.bind.annotation.DeleteMapping
import org.springframework.web.bind.annotation.GetMapping
import org.springframework.web.bind.annotation.PathVariable
import org.springframework.web.bind.annotation.RequestMapping
import org.springframework.web.bind.annotation.RestController

@RestController
@Profile("!server")
@RequestMapping("/api/jobs")
class JobsController(private val jobService: JobService) {
    @GetMapping("/{id}")
    fun status(@PathVariable id: String): JobStatusDto = jobService.status(id)

    @GetMapping("/{id}/artifacts/{name}")
    fun artifact(@PathVariable id: String, @PathVariable name: String): ResponseEntity<FileSystemResource> =
        attachment(jobService.artifact(id, name))

    /** Артефакт проверки нарезкой — по id самой упаковки (id внутренней задачи не раскрывается). */
    @GetMapping("/{id}/check/artifacts/{name}")
    fun checkArtifact(@PathVariable id: String, @PathVariable name: String): ResponseEntity<FileSystemResource> =
        attachment(jobService.checkArtifact(id, name))

    /** Вход pack-джоба (загруженный файл) для скачивания. */
    @GetMapping("/{id}/input")
    fun input(@PathVariable id: String): ResponseEntity<FileSystemResource> = attachment(jobService.input(id))

    @DeleteMapping("/{id}")
    fun delete(@PathVariable id: String): ResponseEntity<Void> {
        jobService.delete(id)
        return ResponseEntity.noContent().build()
    }

    private fun attachment(file: LocalDownload): ResponseEntity<FileSystemResource> {
        val headers = HttpHeaders().apply {
            contentDisposition = ContentDisposition.attachment().filename(file.filename, Charsets.UTF_8).build()
            contentType = file.mediaType
        }
        return ResponseEntity.ok().headers(headers).body(FileSystemResource(file.path))
    }
}
