package com.goatpacker.web.service

import com.goatpacker.web.config.AppProperties
import com.goatpacker.web.dto.CalibrationJobForm
import com.goatpacker.web.dto.JobCreatedDto
import com.goatpacker.web.exception.BadRequestException
import com.goatpacker.web.job.CalibrationParams
import com.goatpacker.web.job.Job
import com.goatpacker.web.job.JobKind
import com.goatpacker.web.job.JobRunner
import com.goatpacker.web.job.JobStore
import org.slf4j.LoggerFactory
import org.springframework.stereotype.Service
import java.nio.file.Files
import java.time.OffsetDateTime
import java.util.UUID
import java.util.concurrent.RejectedExecutionException

/**
 * Приём calibration-джобов (тест времени смены прутка AMS): проект пользователя служит шаблоном,
 * поэтому проверяется так же, как вход упаковки (zip, модель, принтер = профиль). Профиль без
 * filament_change_seconds отсекается заранее — калибровать ему нечего.
 */
@Service
@org.springframework.context.annotation.Profile("!server")
class CalibrationService(
    private val properties: AppProperties,
    private val profileService: ProfileService,
    private val store: JobStore,
    private val jobRunner: JobRunner,
    private val projects: Project3mf,
) {
    companion object {
        private val log = LoggerFactory.getLogger(CalibrationService::class.java)
    }

    fun create(form: CalibrationJobForm): JobCreatedDto {
        val file = form.file
        if (file == null || file.isEmpty) {
            throw BadRequestException("Файл .3mf обязателен")
        }
        val fileName = file.originalFilename ?: ""
        if (!fileName.lowercase().endsWith(".3mf")) {
            throw BadRequestException("Файл должен иметь расширение .3mf, получено: «$fileName»")
        }
        val profileId = form.profileId!!.trim()
        val profile = profileService.require(profileId)
        profileService.requireCalibrationSupported(profile)
        profileService.requireMatches(projects.inspect(file), profile)

        val params = CalibrationParams(
            profileId = profileId,
            profilePath = profile.file,
            variant = form.variant ?: "normal",
        )
        val jobId = UUID.randomUUID().toString()
        val job = Job(
            id = jobId,
            kind = JobKind.CALIBRATION,
            directory = properties.jobsRootPath.resolve(jobId),
            createdAt = OffsetDateTime.now(),
        )
        Files.createDirectories(job.directory)
        try {
            file.transferTo(job.directory.resolve("input.3mf").toFile())
            store.put(job)
            jobRunner.runCalibration(job, params)
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

        log.info("Создана calibration-джоба {} (профиль {}, вариант {})", job.id, profileId, params.variant)
        return JobCreatedDto(job.id)
    }
}
