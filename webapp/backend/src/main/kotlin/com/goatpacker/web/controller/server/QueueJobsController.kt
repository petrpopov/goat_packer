package com.goatpacker.web.controller.server

import com.goatpacker.web.dto.AccessRequest
import com.goatpacker.web.dto.CreatedJobDto
import com.goatpacker.web.dto.DownloadLinkDto
import com.goatpacker.web.dto.HealthDto
import com.goatpacker.web.dto.JobRequest
import com.goatpacker.web.dto.ProfileDto
import com.goatpacker.web.dto.QueueJobDetailDto
import com.goatpacker.web.dto.QueueJobPageDto
import com.goatpacker.web.dto.ShareLinkDto
import com.goatpacker.web.security.Actor
import com.goatpacker.web.service.ProfileService
import com.goatpacker.web.service.server.FileDownload
import com.goatpacker.web.service.server.HealthService
import com.goatpacker.web.service.server.JobAccessService
import com.goatpacker.web.service.server.JobCancellationService
import com.goatpacker.web.service.server.JobCreationService
import com.goatpacker.web.service.server.JobFileService
import com.goatpacker.web.service.server.JobViewService
import org.springframework.context.annotation.Profile
import org.springframework.http.ContentDisposition
import org.springframework.http.HttpHeaders
import org.springframework.http.ResponseEntity
import org.springframework.security.core.annotation.AuthenticationPrincipal
import org.springframework.web.bind.annotation.DeleteMapping
import org.springframework.web.bind.annotation.GetMapping
import org.springframework.web.bind.annotation.PathVariable
import org.springframework.web.bind.annotation.PostMapping
import org.springframework.web.bind.annotation.RequestBody
import org.springframework.web.bind.annotation.RequestHeader
import org.springframework.web.bind.annotation.RequestMapping
import org.springframework.web.bind.annotation.RequestParam
import org.springframework.web.bind.annotation.RestController

/** API очереди задач серверного режима: постановка, статус, проверка нарезкой, доступ по ссылке и скачивание. */
@RestController
@Profile("server")
@RequestMapping("/api/v2")
class QueueJobsController(
    private val healthService: HealthService,
    private val profileService: ProfileService,
    private val creation: JobCreationService,
    private val cancellation: JobCancellationService,
    private val views: JobViewService,
    private val access: JobAccessService,
    private val files: JobFileService,
) {
    @GetMapping("/health")
    fun health(): HealthDto = healthService.status()

    @GetMapping("/profiles")
    fun profiles(): List<ProfileDto> = profileService.list()

    @PostMapping("/access")
    fun grantAccess(@RequestBody body: AccessRequest, @AuthenticationPrincipal actor: Actor) =
        access.grantAccess(body.jobId, actor.id, body.token)

    @PostMapping("/jobs")
    fun create(
        @RequestBody body: JobRequest,
        @RequestHeader(IDEMPOTENCY_KEY) key: String,
        @AuthenticationPrincipal actor: Actor,
    ): CreatedJobDto = creation.create(actor.id, key, body)

    /** «Проверить нарезкой» внутри этой же задачи; ответ — id самой задачи (id внутренней проверки наружу не уходит). */
    @PostMapping("/jobs/{id}/check")
    fun createCheck(
        @PathVariable id: String,
        @RequestHeader(IDEMPOTENCY_KEY) key: String,
        @AuthenticationPrincipal actor: Actor,
    ): CreatedJobDto = creation.createCheck(actor.id, key, id)

    @PostMapping("/jobs/{id}/check/cancel")
    fun cancelCheck(@PathVariable id: String, @AuthenticationPrincipal actor: Actor) = cancellation.cancelCheck(id, actor.id)

    @GetMapping("/jobs")
    fun list(@RequestParam(defaultValue = "0") page: Int, @AuthenticationPrincipal actor: Actor): QueueJobPageDto =
        views.list(actor.id, page)

    @GetMapping("/jobs/{id}")
    fun get(
        @PathVariable id: String,
        @RequestParam(defaultValue = "false") debug: Boolean,
        @RequestHeader(name = JOB_TOKEN, required = false) token: String?,
        @AuthenticationPrincipal actor: Actor,
    ): QueueJobDetailDto = views.get(actor.id, id, token, debug)

    @PostMapping("/jobs/{id}/share")
    fun share(@PathVariable id: String, @AuthenticationPrincipal actor: Actor) = ShareLinkDto(access.createShareLink(id, actor.id))

    @DeleteMapping("/jobs/{id}")
    fun delete(@PathVariable id: String, @AuthenticationPrincipal actor: Actor) = cancellation.delete(id, actor.id)

    @PostMapping("/jobs/{id}/artifacts/{artifact}/download")
    fun artifactLink(
        @PathVariable id: String,
        @PathVariable artifact: String,
        @RequestHeader(name = JOB_TOKEN, required = false) token: String?,
        @AuthenticationPrincipal actor: Actor,
    ): DownloadLinkDto = files.artifactLink(actor.id, id, artifact, token)

    @GetMapping("/jobs/{id}/artifacts/{artifact}/file")
    fun artifactFile(@PathVariable id: String, @PathVariable artifact: String, @AuthenticationPrincipal actor: Actor) =
        attachment(files.artifactFile(actor.id, id, artifact))

    @PostMapping("/jobs/{id}/input/download")
    fun inputLink(
        @PathVariable id: String,
        @RequestHeader(name = JOB_TOKEN, required = false) token: String?,
        @AuthenticationPrincipal actor: Actor,
    ): DownloadLinkDto = files.inputLink(actor.id, id, token)

    @GetMapping("/jobs/{id}/input/file")
    fun inputFile(@PathVariable id: String, @AuthenticationPrincipal actor: Actor) = attachment(files.inputFile(actor.id, id))

    private fun attachment(file: FileDownload): ResponseEntity<ByteArray> {
        val disposition = ContentDisposition.attachment().filename(file.filename, Charsets.UTF_8).build().toString()
        return ResponseEntity.ok().header(HttpHeaders.CONTENT_DISPOSITION, disposition).body(file.bytes)
    }

    private companion object {
        const val IDEMPOTENCY_KEY = "Idempotency-Key"
        const val JOB_TOKEN = "X-Job-Token"
    }
}
