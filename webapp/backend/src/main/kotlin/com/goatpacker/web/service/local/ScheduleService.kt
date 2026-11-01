package com.goatpacker.web.service.local

import com.goatpacker.web.config.AppProperties
import com.goatpacker.web.dto.JobCreatedDto
import com.goatpacker.web.dto.ScheduleJobForm
import com.goatpacker.web.exception.BadRequestException
import com.goatpacker.web.model.Job
import com.goatpacker.web.model.JobKind
import com.goatpacker.web.model.JobStatus
import com.goatpacker.web.model.ScheduleDefaults
import com.goatpacker.web.model.ScheduleParams
import com.goatpacker.web.model.ScheduleSource
import com.goatpacker.web.repository.JobStore
import com.goatpacker.web.service.ClockTime
import com.goatpacker.web.service.ProfileService
import com.goatpacker.web.service.Project3mf
import com.goatpacker.web.service.StlInputs
import com.goatpacker.web.service.runner.JobArtifacts
import com.goatpacker.web.service.runner.JobRunner
import com.goatpacker.web.util.SECONDS_PER_MINUTE
import org.slf4j.LoggerFactory
import org.springframework.context.annotation.Profile
import org.springframework.stereotype.Service
import java.nio.file.Files
import java.nio.file.Path
import java.time.OffsetDateTime
import java.util.UUID

/**
 * Приём schedule-джобов. Источник — либо загруженный .3mf, либо артефакт packed.3mf успешного
 * pack-джоба. Проверяет порядок окон «день < ночь < горизонт» и ставит джоб в очередь.
 */
@Service
@Profile("!server")
class ScheduleService(
    private val properties: AppProperties,
    private val profileService: ProfileService,
    private val store: JobStore,
    private val submitter: LocalJobSubmitter,
    private val jobRunner: JobRunner,
    private val projects: Project3mf,
) {
    private val log = LoggerFactory.getLogger(javaClass)

    fun create(form: ScheduleJobForm): JobCreatedDto {
        val profileId = form.profileId!!.trim()
        val jobId = UUID.randomUUID().toString()
        val directory = properties.jobsRootPath.resolve(jobId)
        // Каталожный профиль указывает слайсер runner-образа: джоб получает копию с локальным слайсером.
        val (profile, profileCopy) = profileService.forLocalJob(profileService.require(profileId), directory)

        val source = resolveSource(form)
        if (source is ScheduleSource.Uploaded) profileService.requireMatches(projects.inspect(form.file!!), profile)
        if (source is ScheduleSource.FromPackJob && source.profileId != profileId) {
            throw BadRequestException("Профиль расписания должен совпадать с профилем исходной упаковки")
        }
        val dayStart = form.dayStart?.trim() ?: ScheduleDefaults.DAY_START
        val nightStart = form.nightStart?.trim() ?: ScheduleDefaults.NIGHT_START
        val horizonEnd = form.horizonEnd?.trim() ?: ScheduleDefaults.HORIZON_END
        validateTimeWindow(dayStart, nightStart, horizonEnd)
        val amsCorrection = form.amsCorrection == true
        profileService.requireAmsCorrectionSupported(profile, amsCorrection)

        val params = ScheduleParams(
            profileId = profileId,
            profilePath = profile.file,
            source = source,
            dayStart = dayStart,
            nightStart = nightStart,
            horizonEnd = horizonEnd,
            turnaroundMin = form.turnaroundMin ?: ScheduleDefaults.TURNAROUND_MIN,
            prepMin = form.prepMin ?: ScheduleDefaults.PREP_MIN,
            benchMode = form.benchMode?.trim() ?: ScheduleDefaults.BENCH_MODE_ADAPTIVE,
            amsCorrection = amsCorrection,
            changeSeconds = form.changeSeconds,
        )
        val job = Job(id = jobId, kind = JobKind.SCHEDULE, directory = directory, createdAt = OffsetDateTime.now())
        submitter.submit(
            job,
            prepare = {
                profileCopy?.let { Files.writeString(profile.file, it) }
                copySource(source, form, directory.resolve(StlInputs.INPUT_3MF))
            },
            // Each queued job owns its source; deleting/expiring a pack cannot break it later.
            start = { jobRunner.runSchedule(job, params) },
        )

        log.info("Создана schedule-джоба {} (профиль {}, источник {})", job.id, profileId, source)
        return JobCreatedDto(job.id)
    }

    private fun copySource(source: ScheduleSource, form: ScheduleJobForm, input: Path) {
        when (source) {
            ScheduleSource.Uploaded -> form.file!!.transferTo(input.toFile())
            is ScheduleSource.FromPackJob -> synchronized(source.job) {
                if (source.job.cancelled) throw BadRequestException("Исходная pack-задача удалена")
                Files.copy(source.packedPath, input)
            }
        }
    }

    private fun resolveSource(form: ScheduleJobForm): ScheduleSource {
        val file = form.file
        if (file != null && !form.packJobId.isNullOrBlank()) throw BadRequestException("Укажите только один источник: file или packJobId")
        if (file != null && file.isEmpty) throw BadRequestException("Файл .3mf пуст")
        if (file != null) {
            val fileName = file.originalFilename.orEmpty()
            if (!fileName.lowercase().endsWith(".3mf")) {
                throw BadRequestException("Файл должен иметь расширение .3mf, получено: «$fileName»")
            }
            return ScheduleSource.Uploaded
        }

        val packJobId = form.packJobId?.trim()
        if (packJobId.isNullOrBlank()) throw BadRequestException("Нужен файл .3mf либо packJobId успешной pack-джобы")
        val packJob = store.get(packJobId) ?: throw BadRequestException("Pack-джоба «$packJobId» не найдена")
        if (packJob.kind != JobKind.PACK) throw BadRequestException("Джоба «$packJobId» — не pack-джоба")
        if (packJob.current.status != JobStatus.SUCCEEDED) throw BadRequestException("Pack-джоба «$packJobId» не завершена успешно")
        val packed = packJob.directory.resolve(JobArtifacts.PACKED_3MF)
        if (!Files.exists(packed)) throw BadRequestException("У pack-джобы «$packJobId» нет артефакта ${JobArtifacts.PACKED_3MF}")

        return ScheduleSource.FromPackJob(packed, packJob.packResult?.profileId.orEmpty(), packJob)
    }

    private fun validateTimeWindow(dayStart: String, nightStart: String, horizonEnd: String) {
        val day = toSeconds(dayStart, "dayStart")
        val night = toSeconds(nightStart, "nightStart")
        val horizon = toSeconds(horizonEnd, "horizonEnd")
        if (!(day < night && night < horizon)) throw BadRequestException("окна времени должны удовлетворять день < ночь < горизонт")
    }

    private fun toSeconds(raw: String, field: String): Long {
        val clock = ClockTime.parse(raw)
            ?: throw BadRequestException("$field должен быть в формате ЧЧ:ММ, часы 0..${ClockTime.MAX_HOURS} (получено «$raw»)")
        if (clock.hours > ClockTime.MAX_HOURS) {
            throw BadRequestException("$field: часы должны быть в диапазоне 0..${ClockTime.MAX_HOURS} (получено «$raw»)")
        }
        return clock.totalMinutes * SECONDS_PER_MINUTE
    }
}
