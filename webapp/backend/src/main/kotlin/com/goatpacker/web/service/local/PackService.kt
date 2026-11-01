package com.goatpacker.web.service.local

import com.fasterxml.jackson.databind.ObjectMapper
import com.goatpacker.web.config.AppProperties
import com.goatpacker.web.dto.CustomBed
import com.goatpacker.web.dto.JobCreatedDto
import com.goatpacker.web.dto.PackJobForm
import com.goatpacker.web.exception.BadRequestException
import com.goatpacker.web.model.BuiltProfile
import com.goatpacker.web.model.Inspection
import com.goatpacker.web.model.Job
import com.goatpacker.web.model.JobKind
import com.goatpacker.web.model.PackDefaults
import com.goatpacker.web.model.PackParams
import com.goatpacker.web.model.ResolvedProfile
import com.goatpacker.web.service.CustomProfile
import com.goatpacker.web.service.NozzleCleanSpec
import com.goatpacker.web.service.ProfileService
import com.goatpacker.web.service.Project3mf
import com.goatpacker.web.service.StlInputs
import com.goatpacker.web.service.runner.JobArtifacts
import com.goatpacker.web.service.runner.JobRunner
import org.slf4j.LoggerFactory
import org.springframework.context.annotation.Profile
import org.springframework.stereotype.Service
import org.springframework.web.multipart.MultipartFile
import java.nio.file.Files
import java.nio.file.Path
import java.time.OffsetDateTime
import java.util.UUID

/**
 * Приём pack-джобов: валидация формы, сохранение входа в каталог джоба, постановка в однопоточную очередь
 * [JobRunner]. Упаковка выедает все ядра, поэтому второй джоб ждёт.
 */
@Service
@Profile("!server")
class PackService(
    private val properties: AppProperties,
    private val profileService: ProfileService,
    private val submitter: LocalJobSubmitter,
    private val jobRunner: JobRunner,
    private val projects: Project3mf,
    private val objectMapper: ObjectMapper,
) {
    private val log = LoggerFactory.getLogger(javaClass)

    /** Профиль джоба: [resolved] указывает путь для команды, а [json] при [needsCopy] нужно записать в каталог джоба. */
    private class PreparedProfile(val resolved: ResolvedProfile, val json: String, val needsCopy: Boolean)

    fun create(form: PackJobForm): JobCreatedDto {
        val file = form.file
        if (file == null || file.isEmpty) throw BadRequestException("Файл .3mf или .stl обязателен")
        val fileName = file.originalFilename.orEmpty()
        if (!StlInputs.isAcceptedUpload(fileName)) {
            throw BadRequestException("${StlInputs.UPLOAD_EXTENSIONS_MESSAGE}, получено: «$fileName»")
        }
        val isStl = StlInputs.isStl(fileName)
        val customBed = customBed(form)
        val profileId = form.profileId?.trim().orEmpty()
        if ((customBed == null) == profileId.isEmpty()) {
            throw BadRequestException(if (customBed == null) "Выберите принтер или укажите свой стол" else "Укажите либо принтер из каталога, либо свой стол")
        }
        // STL — сетка без настроек: проверять в нём нечего, принтер — выбор пользователя (любой из каталога).
        val inspection = if (isStl) {
            StlInputs.requireValidStl(file)
            null
        } else {
            projects.inspect(file)
        }

        val jobId = UUID.randomUUID().toString()
        val directory = properties.jobsRootPath.resolve(jobId)
        val prepared = prepareProfile(profileId, customBed, inspection, directory)
        val profile = prepared.resolved
        val sourceCommand = if (isStl) stlSourceCommand(prepared.json) else null
        val nozzleClean = form.nozzleClean?.trim().orEmpty()
        validateNozzleClean(nozzleClean, profile)
        val amsCorrection = form.amsCorrection == true
        profileService.requireAmsCorrectionSupported(profile, amsCorrection)

        val params = PackParams(
            profileId = profile.dto.id,
            profilePath = profile.file,
            clearance = form.clearance ?: PackDefaults.CLEARANCE_MM,
            resolution = form.resolution ?: PackDefaults.RESOLUTION_MM,
            attempts = form.attempts ?: PackDefaults.ATTEMPTS,
            candidates = form.candidates ?: PackDefaults.CANDIDATES,
            seed = form.seed ?: PackDefaults.SEED,
            edgeMargin = form.edgeMargin,
            layout = form.layout ?: PackDefaults.LAYOUT,
            nozzleClean = nozzleClean,
            timeWindowHours = form.timeWindowHours,
            amsCorrection = amsCorrection,
            changeSeconds = form.changeSeconds,
            timeMarginPct = form.timeMarginPct,
            supportOverlapMm = form.supportOverlapMm,
            sourceCommand = sourceCommand,
            finalSlice = form.finalSlice ?: true,
            customBed = customBed,
        )
        val job = Job(id = jobId, kind = JobKind.PACK, directory = directory, createdAt = OffsetDateTime.now())
        submitter.submit(
            job,
            prepare = {
                file.transferTo(directory.resolve(StlInputs.inputName(fileName)).toFile())
                if (prepared.needsCopy) Files.writeString(params.profilePath, prepared.json)
                job.packParams = params
                job.originalName = fileName.substringAfterLast('/').substringAfterLast('\\')
                job.profileDto = profile.dto
            },
            start = { jobRunner.runPack(job, params) },
        )

        log.info("Создана pack-джоба {} (профиль {}, файл {} байт)", job.id, profile.dto.id, file.size)
        return JobCreatedDto(job.id)
    }

    /**
     * Профиль джоба. Свой стол: разовый профиль — profile.json в каталоге джоба; проект из STL на своём столе
     * source_project собирает с printer_model «Custom Printer». Каталожные профили (printer_catalog) указывают
     * слайсер runner-образа: локально джоб получает копию со своим слайсером.
     */
    private fun prepareProfile(profileId: String, customBed: CustomBed?, inspection: Inspection?, directory: Path): PreparedProfile {
        val jobProfileFile = directory.resolve(JobArtifacts.PROFILE_FILE)
        if (customBed != null) {
            val built = customProfile(customBed, inspection)
            return PreparedProfile(ResolvedProfile(built.dto, jobProfileFile), built.json, needsCopy = true)
        }
        val catalog = profileService.require(profileId)
        if (inspection != null) profileService.requireMatches(inspection, catalog)
        val localCopy = profileService.localSlicerCopy(catalog)
            ?: return PreparedProfile(catalog, Files.readString(catalog.file), needsCopy = false)
        return PreparedProfile(ResolvedProfile(catalog.dto, jobProfileFile), localCopy, needsCopy = true)
    }

    private fun customProfile(customBed: CustomBed, inspection: Inspection?): BuiltProfile {
        if (inspection == null) return profileService.custom(customBed, null, CustomProfile.NAME)
        projects.requireModel(inspection)
        // Распознанный принтер упаковывается своим каталожным профилем: разовый стол спорил бы с ним.
        if (profileService.detect(inspection).profileId != null) throw BadRequestException(CustomProfile.RECOGNISED_PRINTER)
        return profileService.custom(customBed, inspection.projectBed, inspection.printerModel)
    }

    private fun stlSourceCommand(profileJson: String): List<String> {
        val node = objectMapper.readTree(profileJson)
        val (presets, slicer) = StlInputs.localTools(properties, node)
        return StlInputs.command(properties.sourceProjectBinPath.toString(), presets, slicer, node)
    }

    /** Свой стол из полей custom*: все три размера или ни одного (отступ без размеров — ошибка формы). */
    private fun customBed(form: PackJobForm): CustomBed? {
        val sizes = listOf(form.customWidth, form.customDepth, form.customHeight)
        if (sizes.all { it == null } && form.customMargin == null) return null
        if (sizes.any { it == null }) throw BadRequestException("Для своего стола нужны ширина, глубина и высота")
        return CustomBed(form.customWidth!!, form.customDepth!!, form.customHeight!!, form.customMargin ?: CustomBed.DEFAULT_MARGIN_MM)
            .also { it.validate() }
    }

    /**
     * Формат спеки проверяется регуляркой; активная чистка дополнительно требует, чтобы профиль
     * принтера реально содержал код чистки (nozzle_clean_gcode) — у A2L и U1 его нет, и вместо
     * ошибки от Rust-бинарника на пятой минуте слайсинга пользователь получает понятные 400.
     */
    private fun validateNozzleClean(spec: String, profile: ResolvedProfile) {
        if (NozzleCleanSpec.isOff(spec)) return
        if (!profile.dto.supportsNozzleClean) {
            throw BadRequestException("Принтер «${profile.dto.name}» не поддерживает чистку сопла — в профиле нет кода очистки")
        }
        if (!NozzleCleanSpec.PATTERN.matches(spec)) {
            throw BadRequestException(
                "nozzleClean: допустимо \"\", \"off\", \"global\", \"global,every=N\", \"from-z=H\", \"from-z=H,every=N\", " +
                    "где H — высота в мм, N — целое ≥ 1",
            )
        }
        val every = NozzleCleanSpec.every(spec)
        if (every.isNotEmpty() && every.toLongOrNull() == null) throw BadRequestException("nozzleClean: every слишком велико")
        val fromZ = NozzleCleanSpec.fromZ(spec)
        if (fromZ != null && (!fromZ.isFinite() || fromZ <= 0.0)) throw BadRequestException("nozzleClean: from-z=H требует H > 0")
    }
}
