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
import java.nio.file.Files
import java.time.OffsetDateTime
import java.util.UUID
import java.util.concurrent.RejectedExecutionException

/**
 * Приём pack-джобов: валидация формы, сохранение input.3mf в каталог джоба, постановка в
 * однопоточную очередь [JobRunner]. Упаковка выедает все ядра, поэтому второй джоб ждёт.
 */
@Service
class PackService(
    private val properties: AppProperties,
    private val profileService: ProfileService,
    private val store: JobStore,
    private val jobRunner: JobRunner,
) {
    companion object {
        private val log = LoggerFactory.getLogger(PackService::class.java)

        /** "", "off", "global", "global,every=N", "from-z=H", "from-z=H,every=N" (H в мм, N ≥ 1). */
        private val NOZZLE_CLEAN_SPEC = Regex(
            """^(global|global,every=[1-9]\d*|from-z=(\d+(\.\d+)?))(,every=[1-9]\d*)?$""",
        )
    }

    fun create(form: PackJobForm): JobCreatedDto {
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
        val nozzleClean = form.nozzleClean?.trim() ?: ""
        validateNozzleClean(nozzleClean, profile)

        val params = PackParams(
            profileId = profileId,
            profilePath = profile.file,
            clearance = form.clearance ?: 0.7,
            resolution = form.resolution ?: 0.25,
            // Дефолт = «Сильному» уровню ползунка UI (консольные 4800/2400, валидированы на
            // козочках): поиск останавливается сам при плато, так что потолок почти бесплатен,
            // а слабый дефолт стабильно недоупаковывал на 1 объект.
            attempts = form.attempts ?: 4800,
            candidates = form.candidates ?: 2400,
            // Seed без значения — случайный на прогон: результат поиска сильно зависит от
            // seed (goat1 P1S: seed 20270831 ни разу не находит 18 даже за 4800 полных
            // попыток, другой seed находит на 387-й), поэтому «повторить запуск» — это
            // легитимный способ улучшить плотность. Закреплённый seed = воспроизводимость.
            seed = form.seed ?: kotlin.random.Random.nextLong(0, Long.MAX_VALUE),
            nozzleClean = nozzleClean,
        )

        val jobId = UUID.randomUUID().toString()
        val job = Job(
            id = jobId,
            kind = JobKind.PACK,
            directory = properties.jobsRootPath.resolve(jobId),
            createdAt = OffsetDateTime.now(),
        )
        Files.createDirectories(job.directory)
        file.transferTo(job.directory.resolve("input.3mf").toFile())
        store.put(job)

        try {
            jobRunner.runPack(job, params)
        } catch (ex: RejectedExecutionException) {
            store.remove(job.id)
            throw BadRequestException("Очередь задач переполнена — попробуйте повторить позже")
        }

        log.info("Создана pack-джоба {} (профиль {}, файл {} байт)", job.id, profileId, file.size)
        return JobCreatedDto(job.id)
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
        val fromZ = Regex("""from-z=(\d+(\.\d+)?)""").find(spec)?.groupValues?.get(1)?.toDoubleOrNull()
        if (fromZ != null && fromZ <= 0.0) {
            throw BadRequestException("nozzleClean: from-z=H требует H > 0")
        }
    }
}
