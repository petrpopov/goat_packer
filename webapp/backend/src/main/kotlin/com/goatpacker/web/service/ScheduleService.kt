package com.goatpacker.web.service

import com.goatpacker.web.config.AppProperties
import com.goatpacker.web.dto.JobCreatedDto
import com.goatpacker.web.dto.ScheduleJobForm
import com.goatpacker.web.exception.BadRequestException
import com.goatpacker.web.job.Job
import com.goatpacker.web.job.JobKind
import com.goatpacker.web.job.JobRunner
import com.goatpacker.web.job.JobStatus
import com.goatpacker.web.job.JobStore
import com.goatpacker.web.job.ScheduleParams
import com.goatpacker.web.job.ScheduleSource
import org.slf4j.LoggerFactory
import org.springframework.stereotype.Service
import java.nio.file.Files
import java.time.OffsetDateTime
import java.util.UUID
import java.util.concurrent.RejectedExecutionException

/**
 * Приём schedule-джобов. Источник — либо загруженный .3mf, либо артефакт packed.3mf успешного
 * pack-джоба. Проверяет порядок окон «день < ночь < горизонт» и ставит джоб в очередь.
 */
@Service
class ScheduleService(
    private val properties: AppProperties,
    private val profileService: ProfileService,
    private val store: JobStore,
    private val jobRunner: JobRunner,
) {
    companion object {
        private val log = LoggerFactory.getLogger(ScheduleService::class.java)

        private val TIME_PATTERN = Regex("""^(\d{1,2}):([0-5]\d)$""")
    }

    fun create(form: ScheduleJobForm): JobCreatedDto {
        val profileId = form.profileId!!.trim()
        val profile = profileService.require(profileId)

        val source = resolveSource(form)
        val dayStart = form.dayStart?.trim() ?: "10:00"
        val nightStart = form.nightStart?.trim() ?: "23:00"
        val horizonEnd = form.horizonEnd?.trim() ?: "34:00"
        validateTimeWindow(dayStart, nightStart, horizonEnd)

        val params = ScheduleParams(
            profileId = profileId,
            profilePath = profile.file,
            source = source,
            dayStart = dayStart,
            nightStart = nightStart,
            horizonEnd = horizonEnd,
            turnaroundMin = form.turnaroundMin ?: 10,
            prepMin = form.prepMin ?: 10,
            benchMode = form.benchMode?.trim() ?: "adaptive",
        )

        val jobId = UUID.randomUUID().toString()
        val job = Job(
            id = jobId,
            kind = JobKind.SCHEDULE,
            directory = properties.jobsRootPath.resolve(jobId),
            createdAt = OffsetDateTime.now(),
        )
        Files.createDirectories(job.directory)
        if (source is ScheduleSource.Uploaded) {
            form.file!!.transferTo(job.directory.resolve("input.3mf").toFile())
        }
        store.put(job)

        try {
            jobRunner.runSchedule(job, params)
        } catch (ex: RejectedExecutionException) {
            store.remove(job.id)
            throw BadRequestException("Очередь задач переполнена — попробуйте повторить позже")
        }

        log.info("Создана schedule-джоба {} (профиль {}, источник {})", job.id, profileId, source)
        return JobCreatedDto(job.id)
    }

    private fun resolveSource(form: ScheduleJobForm): ScheduleSource {
        val file = form.file
        if (file != null && !file.isEmpty) {
            val fileName = file.originalFilename ?: ""
            if (!fileName.lowercase().endsWith(".3mf")) {
                throw BadRequestException("Файл должен иметь расширение .3mf, получено: «$fileName»")
            }

            return ScheduleSource.Uploaded
        }

        val packJobId = form.packJobId?.trim()
        if (packJobId.isNullOrBlank()) {
            throw BadRequestException("Нужен файл .3mf либо packJobId успешной pack-джобы")
        }

        val packJob = store.get(packJobId)
            ?: throw BadRequestException("Pack-джоба «$packJobId» не найдена")
        if (packJob.kind != JobKind.PACK) {
            throw BadRequestException("Джоба «$packJobId» — не pack-джоба")
        }
        if (packJob.current.status != JobStatus.SUCCEEDED) {
            throw BadRequestException("Pack-джоба «$packJobId» не завершена успешно")
        }
        val packed = packJob.directory.resolve("packed.3mf")
        if (!Files.exists(packed)) {
            throw BadRequestException("У pack-джобы «$packJobId» нет артефакта packed.3mf")
        }

        return ScheduleSource.FromPackJob(packed)
    }

    private fun validateTimeWindow(dayStart: String, nightStart: String, horizonEnd: String) {
        val day = toSeconds(dayStart, "dayStart")
        val night = toSeconds(nightStart, "nightStart")
        val horizon = toSeconds(horizonEnd, "horizonEnd")
        if (!(day < night && night < horizon)) {
            throw BadRequestException("окна времени должны удовлетворять день < ночь < горизонт")
        }
    }

    private fun toSeconds(raw: String, field: String): Long {
        val match = TIME_PATTERN.matchEntire(raw)
            ?: throw BadRequestException("$field должен быть в формате ЧЧ:ММ, часы 0..47 (получено «$raw»)")
        val hours = match.groupValues[1].toLong()
        val minutes = match.groupValues[2].toLong()
        if (hours > 47) {
            throw BadRequestException("$field: часы должны быть в диапазоне 0..47 (получено «$raw»)")
        }

        return hours * 3600 + minutes * 60
    }
}
