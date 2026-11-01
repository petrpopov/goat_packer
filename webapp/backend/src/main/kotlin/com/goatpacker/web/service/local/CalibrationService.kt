package com.goatpacker.web.service.local

import com.goatpacker.web.config.AppProperties
import com.goatpacker.web.dto.CalibrationJobForm
import com.goatpacker.web.dto.JobCreatedDto
import com.goatpacker.web.exception.BadRequestException
import com.goatpacker.web.model.CalibrationParams
import com.goatpacker.web.model.Job
import com.goatpacker.web.model.JobKind
import com.goatpacker.web.model.ScheduleDefaults
import com.goatpacker.web.service.ProfileService
import com.goatpacker.web.service.Project3mf
import com.goatpacker.web.service.StlInputs
import com.goatpacker.web.service.runner.JobRunner
import org.slf4j.LoggerFactory
import org.springframework.context.annotation.Profile
import org.springframework.stereotype.Service
import java.nio.file.Files
import java.time.OffsetDateTime
import java.util.UUID

/**
 * Приём calibration-джобов (тест времени смены прутка AMS): проект пользователя служит шаблоном,
 * поэтому проверяется так же, как вход упаковки (zip, модель, принтер = профиль). Профиль без
 * filament_change_seconds отсекается заранее — калибровать ему нечего.
 */
@Service
@Profile("!server")
class CalibrationService(
    private val properties: AppProperties,
    private val profileService: ProfileService,
    private val submitter: LocalJobSubmitter,
    private val jobRunner: JobRunner,
    private val projects: Project3mf,
) {
    private val log = LoggerFactory.getLogger(javaClass)

    fun create(form: CalibrationJobForm): JobCreatedDto {
        val file = form.file
        if (file == null || file.isEmpty) throw BadRequestException("Файл .3mf обязателен")
        val fileName = file.originalFilename.orEmpty()
        if (!fileName.lowercase().endsWith(".3mf")) throw BadRequestException("Файл должен иметь расширение .3mf, получено: «$fileName»")

        val profileId = form.profileId!!.trim()
        val jobId = UUID.randomUUID().toString()
        val directory = properties.jobsRootPath.resolve(jobId)
        // Каталожный профиль указывает слайсер runner-образа: джоб получает копию с локальным слайсером.
        val (profile, profileCopy) = profileService.forLocalJob(profileService.require(profileId), directory)
        profileService.requireCalibrationSupported(profile)
        profileService.requireMatches(projects.inspect(file), profile)

        val params = CalibrationParams(
            profileId = profileId,
            profilePath = profile.file,
            variant = form.variant ?: ScheduleDefaults.CALIBRATION_VARIANT_NORMAL,
        )
        val job = Job(id = jobId, kind = JobKind.CALIBRATION, directory = directory, createdAt = OffsetDateTime.now())
        submitter.submit(
            job,
            prepare = {
                file.transferTo(directory.resolve(StlInputs.INPUT_3MF).toFile())
                profileCopy?.let { Files.writeString(profile.file, it) }
            },
            start = { jobRunner.runCalibration(job, params) },
        )

        log.info("Создана calibration-джоба {} (профиль {}, вариант {})", job.id, profileId, params.variant)
        return JobCreatedDto(job.id)
    }
}
