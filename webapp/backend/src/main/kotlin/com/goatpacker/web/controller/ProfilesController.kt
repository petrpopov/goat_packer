package com.goatpacker.web.controller

import com.goatpacker.web.dto.DetectProfileDto
import com.goatpacker.web.dto.ProfileDto
import com.goatpacker.web.service.ProfileService
import org.springframework.http.MediaType
import org.springframework.web.bind.annotation.GetMapping
import org.springframework.web.bind.annotation.PostMapping
import org.springframework.web.bind.annotation.RequestMapping
import org.springframework.web.bind.annotation.RequestParam
import org.springframework.web.bind.annotation.RestController
import org.springframework.web.multipart.MultipartFile

@RestController
@org.springframework.context.annotation.Profile("!server")
@RequestMapping("/api/profiles")
class ProfilesController(
    private val profileService: ProfileService,
) {
    @GetMapping
    fun list(): List<ProfileDto> = profileService.list()

    /** Автоопределение профиля по printer_model внутри загруженного 3MF. */
    @PostMapping("/detect", consumes = [MediaType.MULTIPART_FORM_DATA_VALUE])
    fun detect(@RequestParam("file") file: MultipartFile): DetectProfileDto {
        val detection = profileService.detectFrom3mf(file)
        return DetectProfileDto(profileId = detection.profileId, printerModel = detection.printerModel)
    }
}
