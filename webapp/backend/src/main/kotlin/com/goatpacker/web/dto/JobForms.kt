package com.goatpacker.web.dto

import jakarta.validation.constraints.DecimalMax
import jakarta.validation.constraints.DecimalMin
import jakarta.validation.constraints.Max
import jakarta.validation.constraints.Min
import jakarta.validation.constraints.NotBlank
import jakarta.validation.constraints.NotNull
import jakarta.validation.constraints.Pattern
import org.springframework.web.multipart.MultipartFile

/** Форма создания pack-джоба (multipart/form-data). Пустые числовые поля — дефолты CLI. */
data class PackJobForm(
    @field:NotNull(message = "Файл .3mf обязателен")
    val file: MultipartFile? = null,
    @field:NotBlank(message = "profileId обязателен")
    val profileId: String? = null,
    @field:DecimalMin(value = "0.05", message = "clearance должен быть в диапазоне 0.05..10 мм")
    @field:DecimalMax(value = "10", message = "clearance должен быть в диапазоне 0.05..10 мм")
    val clearance: Double? = null,
    @field:DecimalMin(value = "0.05", message = "resolution должен быть в диапазоне 0.05..1.0 мм")
    @field:DecimalMax(value = "1.0", message = "resolution должен быть в диапазоне 0.05..1.0 мм")
    val resolution: Double? = null,
    @field:Min(value = 1, message = "attempts должен быть в диапазоне 1..1000000")
    @field:Max(value = 1000000, message = "attempts должен быть в диапазоне 1..1000000")
    val attempts: Long? = null,
    @field:Min(value = 1, message = "candidates должен быть в диапазоне 1..100000")
    @field:Max(value = 100000, message = "candidates должен быть в диапазоне 1..100000")
    val candidates: Long? = null,
    @field:Min(value = 0, message = "seed должен быть в диапазоне 0..2^63-1")
    val seed: Long? = null,
    @field:DecimalMin(value = "0.0", message = "edgeMargin должен быть в диапазоне 0..20 мм")
    @field:DecimalMax(value = "20", message = "edgeMargin должен быть в диапазоне 0..20 мм")
    val edgeMargin: Double? = null,
    @field:Pattern(
        regexp = """^(search|grid)$""",
        message = "layout допустим search или grid",
    )
    val layout: String? = null,
    val nozzleClean: String? = null,
)

/** Форма создания schedule-джоба (multipart/form-data). */
data class ScheduleJobForm(
    val file: MultipartFile? = null,
    val packJobId: String? = null,
    @field:NotBlank(message = "profileId обязателен")
    val profileId: String? = null,
    @field:Pattern(
        regexp = """^\d{1,2}:[0-5]\d$""",
        message = "dayStart должен быть в формате ЧЧ:ММ, часы 0..47",
    )
    val dayStart: String? = null,
    @field:Pattern(
        regexp = """^\d{1,2}:[0-5]\d$""",
        message = "nightStart должен быть в формате ЧЧ:ММ, часы 0..47",
    )
    val nightStart: String? = null,
    @field:Pattern(
        regexp = """^\d{1,2}:[0-5]\d$""",
        message = "horizonEnd должен быть в формате ЧЧ:ММ, часы 0..47",
    )
    val horizonEnd: String? = null,
    @field:Min(value = 0, message = "turnaroundMin должен быть в диапазоне 0..600")
    @field:Max(value = 600, message = "turnaroundMin должен быть в диапазоне 0..600")
    val turnaroundMin: Int? = null,
    @field:Min(value = 0, message = "prepMin должен быть в диапазоне 0..600")
    @field:Max(value = 600, message = "prepMin должен быть в диапазоне 0..600")
    val prepMin: Int? = null,
    @field:Pattern(
        regexp = """^(adaptive|full)$""",
        message = "benchMode допустим adaptive или full",
    )
    val benchMode: String? = null,
)

/** Ответ /api/profiles/detect: profileId = null, когда по файлу определить не удалось; printerModel — что за принтер записан в файле (для понятных сообщений). */
@com.fasterxml.jackson.annotation.JsonInclude(com.fasterxml.jackson.annotation.JsonInclude.Include.ALWAYS)
data class DetectProfileDto(
    val profileId: String?,
    val printerModel: String?,
)
