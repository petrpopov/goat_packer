package com.goatpacker.web.controller.server

import com.goatpacker.web.dto.UploadDto
import com.goatpacker.web.dto.UploadIntent
import com.goatpacker.web.security.Actor
import com.goatpacker.web.service.server.UploadReceiver
import com.goatpacker.web.service.server.UploadService
import jakarta.servlet.http.HttpServletRequest
import org.springframework.context.annotation.Profile
import org.springframework.security.core.annotation.AuthenticationPrincipal
import org.springframework.web.bind.annotation.DeleteMapping
import org.springframework.web.bind.annotation.GetMapping
import org.springframework.web.bind.annotation.PathVariable
import org.springframework.web.bind.annotation.PostMapping
import org.springframework.web.bind.annotation.PutMapping
import org.springframework.web.bind.annotation.RequestBody
import org.springframework.web.bind.annotation.RequestMapping
import org.springframework.web.bind.annotation.RestController

/** Загрузка файлов: резерв места, передача байт потоком, список и удаление. */
@RestController
@Profile("server")
@RequestMapping("/api/v2/uploads")
class UploadController(
    private val uploads: UploadService,
    private val receiver: UploadReceiver,
) {
    @PostMapping
    fun reserve(@RequestBody intent: UploadIntent, @AuthenticationPrincipal actor: Actor): UploadDto =
        uploads.reserve(actor.id, intent)

    @PutMapping("/{id}/content")
    fun content(@PathVariable id: String, request: HttpServletRequest, @AuthenticationPrincipal actor: Actor): UploadDto =
        receiver.receive(id, actor.id, request.contentLengthLong, request.inputStream)

    @GetMapping
    fun list(@AuthenticationPrincipal actor: Actor): List<UploadDto> = uploads.list(actor.id)

    @DeleteMapping("/{id}")
    fun delete(@PathVariable id: String, @AuthenticationPrincipal actor: Actor) = uploads.delete(id, actor.id)

    @GetMapping("/{id}")
    fun get(@PathVariable id: String, @AuthenticationPrincipal actor: Actor): UploadDto = uploads.get(id, actor.id)
}
