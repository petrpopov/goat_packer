package com.goatpacker.web.service

import com.goatpacker.web.config.AppProperties
import com.goatpacker.web.dto.JobCreatedDto
import com.goatpacker.web.dto.PackJobForm
import com.goatpacker.web.exception.BadRequestException
import com.goatpacker.web.job.Job
import com.goatpacker.web.job.JobKind
import com.goatpacker.web.job.JobRunner
import com.goatpacker.web.job.JobStore
import com.goatpacker.web.job.PackParams
import org.slf4j.LoggerFactory
import org.springframework.stereotype.Service
import org.springframework.web.multipart.MultipartFile
import java.nio.file.Files
import java.time.OffsetDateTime
import java.util.UUID
import java.util.concurrent.RejectedExecutionException

/**
 * Приём pack-джобов: валидация формы, сохранение input.3mf в каталог джоба, постановка в
 * однопоточную очередь [JobRunner]. Упаковка выедает все ядра, поэтому второй джоб ждёт.
 */
@Service
@org.springframework.context.annotation.Profile("!server")
class PackService(
    private val properties: AppProperties,
    private val profileService: ProfileService,
    private val store: JobStore,
    private val jobRunner: JobRunner,
    private val projects: Project3mf,
    private val objectMapper: com.fasterxml.jackson.databind.ObjectMapper,
) {
    companion object {
        private val log = LoggerFactory.getLogger(PackService::class.java)

        /** "", "off", "global", "global,every=N", "from-z=H", "from-z=H,every=N" (H в мм, N ≥ 1). */
        private val NOZZLE_CLEAN_SPEC = Regex(
            """^(global|from-z=\d+(\.\d+)?)(,every=[1-9]\d*)?$""",
        )
    }

    fun create(form: PackJobForm): JobCreatedDto {
        val file = form.file
        if (file == null || file.isEmpty) {
            throw BadRequestException("Файл .3mf или .stl обязателен")
        }
        val fileName = file.originalFilename ?: ""
        if (!StlInputs.isAcceptedUpload(fileName)) {
            throw BadRequestException("${StlInputs.UPLOAD_EXTENSIONS_MESSAGE}, получено: «$fileName»")
        }
        val stl = StlInputs.isStl(fileName)
        val customBed = customBed(form)
        val profileId = form.profileId?.trim().orEmpty()
        if ((customBed == null) == profileId.isEmpty()) {
            throw BadRequestException(if (customBed == null) "Выберите принтер или укажите свой стол" else "Укажите либо принтер из каталога, либо свой стол")
        }
        // STL — сетка без настроек: проверять в нём нечего, принтер — выбор пользователя (любой из каталога).
        val inspection = if (stl) {
            if (file.size > 200L * 1024 * 1024) throw BadRequestException("Файл больше 200 МБ")
            null
        } else {
            projects.inspect(file)
        }
        val jobId = UUID.randomUUID().toString()
        val directory = properties.jobsRootPath.resolve(jobId)
        // Свой стол: разовый профиль — profile.json в каталоге джоба (пишется, когда каталог создан).
        // Проект из STL на своём столе source_project собирает с printer_model «Custom Printer».
        val custom = customBed?.let {
            if (inspection == null) {
                profileService.custom(it, null, CustomProfile.NAME)
            } else {
                projects.requireModel(inspection)
                profileService.custom(it, inspection.projectBed, inspection.printerModel)
            }
        }
        val catalog = if (custom == null) {
            profileService.require(profileId).also { if (inspection != null) profileService.requireMatches(inspection, it) }
        } else {
            null
        }
        // Каталожные профили (printer_catalog) указывают слайсер runner-образа: локально — свой слайсер.
        val localCopy = catalog?.let { localSlicerCopy(it) }
        val profileJson = custom?.json ?: localCopy ?: Files.readString(catalog!!.file)
        val profile = if (catalog != null && localCopy == null) catalog else {
            ProfileService.ResolvedProfile(custom?.dto ?: catalog!!.dto, directory.resolve("profile.json"))
        }
        val sourceCommand = if (stl) {
            val node = objectMapper.readTree(profileJson)
            val (presets, slicer) = StlInputs.localTools(properties, node)
            StlInputs.command(properties.sourceProjectBinPath.toString(), presets, slicer, node)
        } else {
            null
        }
        val nozzleClean = form.nozzleClean?.trim() ?: ""
        validateNozzleClean(nozzleClean, profile)
        val amsCorrection = form.amsCorrection == true
        profileService.requireAmsCorrectionSupported(profile, amsCorrection)

        val params = PackParams(
            profileId = profile.dto.id,
            profilePath = profile.file,
            clearance = form.clearance ?: 0.7,
            resolution = form.resolution ?: 0.25,
            // Дефолт CLI. Фронт поле больше не шлёт: с поддержками поиск всё равно перебирает
            // фиксированные 64 варианта, ползунок усилия ни на что не влиял. Поле принимается
            // ради совместимости API.
            attempts = form.attempts ?: 16_000,
            candidates = form.candidates ?: 2400,
            seed = form.seed ?: 20_270_831,
            edgeMargin = form.edgeMargin,
            layout = form.layout ?: "search",
            nozzleClean = nozzleClean,
            timeWindowHours = form.timeWindowHours,
            amsCorrection = amsCorrection,
            changeSeconds = form.changeSeconds,
            timeMarginPct = form.timeMarginPct,
            supportOverlapMm = form.supportOverlapMm,
            sourceCommand = sourceCommand,
        )

        val job = Job(
            id = jobId,
            kind = JobKind.PACK,
            directory = directory,
            createdAt = OffsetDateTime.now(),
        )
        Files.createDirectories(job.directory)
        try {
            file.transferTo(job.directory.resolve(StlInputs.inputName(fileName)).toFile())
            if (profile.file != catalog?.file) Files.writeString(params.profilePath, profileJson)
            store.put(job)
            jobRunner.runPack(job, params)
        } catch (ex: RejectedExecutionException) {
            store.remove(job.id)
            job.cancel()
            job.directory.toFile().deleteRecursively()
            throw BadRequestException("Очередь задач переполнена — попробуйте повторить позже")
        } catch (ex: Exception) {
            store.remove(job.id)
            job.cancel()
            job.directory.toFile().deleteRecursively()
            throw ex
        }

        log.info("Создана pack-джоба {} (профиль {}, файл {} байт)", job.id, profile.dto.id, file.size)
        return JobCreatedDto(job.id)
    }

    /**
     * Профиль из каталога OrcaSlicer несёт slicer_path runner-образа (/opt/slicers/<kind>/AppRun):
     * локально такого пути нет, и джоб получает копию профиля со слайсером из настроек. Ручные
     * профили (P1S и др. с путями macOS) идут как есть. null — копия не нужна.
     */
    private fun localSlicerCopy(profile: ProfileService.ResolvedProfile): String? {
        val node = objectMapper.readTree(profile.file.toFile()) as com.fasterxml.jackson.databind.node.ObjectNode
        if (!node.path("slicer_path").asText("").startsWith("/opt/slicers/")) return null
        node.put("slicer_path", if (node.path("slicer_kind").asText() == "bambu") properties.bambuSlicer else properties.orcaSlicer)
        return objectMapper.writeValueAsString(node)
    }

    /** Свой стол из полей custom*: все три размера или ни одного (отступ без размеров — ошибка формы). */
    private fun customBed(form: PackJobForm): com.goatpacker.web.dto.CustomBed? {
        val sizes = listOf(form.customWidth, form.customDepth, form.customHeight)
        if (sizes.all { it == null } && form.customMargin == null) return null
        if (sizes.any { it == null }) throw BadRequestException("Для своего стола нужны ширина, глубина и высота")
        return com.goatpacker.web.dto.CustomBed(form.customWidth!!, form.customDepth!!, form.customHeight!!, form.customMargin ?: 5.0)
            .also { it.validate() }
    }

    /**
     * Формат спеки проверяется регуляркой; активная чистка дополнительно требует, чтобы профиль
     * принтера реально содержит код чистки (nozzle_clean_gcode) — у A2L и U1 его нет, и вместо
     * ошибки от Rust-бинарника на пятой минуте слайсинга пользователь получает понятные 400.
     */
    private fun validateNozzleClean(spec: String, profile: ProfileService.ResolvedProfile) {
        if (spec.isEmpty() || spec.equals("off", ignoreCase = true)) {
            return
        }
        if (!profile.dto.supportsNozzleClean) {
            throw BadRequestException(
                "Принтер «${profile.dto.name}» не поддерживает чистку сопла — " +
                    "в профиле нет кода очистки",
            )
        }
        if (!NOZZLE_CLEAN_SPEC.matches(spec)) {
            throw BadRequestException(
                "nozzleClean: допустимо \"\", \"off\", \"global\", \"global,every=N\", " +
                    "\"from-z=H\", \"from-z=H,every=N\", где H — высота в мм, N — целое ≥ 1",
            )
        }
        val every = spec.substringAfter(",every=", "")
        if (every.isNotEmpty() && every.toLongOrNull() == null) {
            throw BadRequestException("nozzleClean: every слишком велико")
        }
        val fromZ = Regex("""from-z=(\d+(\.\d+)?)""").find(spec)?.groupValues?.get(1)?.toDoubleOrNull()
        if (fromZ != null && (!fromZ.isFinite() || fromZ <= 0.0)) {
            throw BadRequestException("nozzleClean: from-z=H требует H > 0")
        }
    }

}
