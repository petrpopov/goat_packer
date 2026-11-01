package com.goatpacker.web.service.local

import com.goatpacker.web.config.AppProperties
import com.goatpacker.web.dto.JobCreatedDto
import com.goatpacker.web.exception.BadRequestException
import com.goatpacker.web.model.Job
import com.goatpacker.web.model.JobKind
import com.goatpacker.web.model.JobStatus
import com.goatpacker.web.repository.JobStore
import com.goatpacker.web.service.StlInputs
import com.goatpacker.web.service.runner.JobArtifacts
import com.goatpacker.web.service.runner.JobRunner
import com.goatpacker.web.util.ProcessKiller
import org.slf4j.LoggerFactory
import org.springframework.context.annotation.Profile
import org.springframework.stereotype.Service
import java.nio.file.Files
import java.nio.file.Path
import java.time.OffsetDateTime
import java.util.UUID

/**
 * «Нарезать стол» (локальный режим): новая внутренняя pack-джоба доводит стол упаковки, собранный без полной
 * нарезки, — тот же вход, профиль и параметры плюс --finish-state по её состоянию. Наружу она живёт как
 * часть упаковки: id внутренней джобы не раскрывается.
 */
@Service
@Profile("!server")
class PackCheckService(
    private val properties: AppProperties,
    private val store: JobStore,
    private val submitter: LocalJobSubmitter,
    private val jobRunner: JobRunner,
) {
    private val log = LoggerFactory.getLogger(javaClass)

    fun start(packJobId: String): JobCreatedDto {
        val parent = requireParent(packJobId)
        // Проверка живёт внутри упаковки: идущую или успешную второй раз не запускаем, упавшую/пропавшую — можно.
        parent.checkJobId?.let { store.get(it) }?.let { previous ->
            when (previous.current.status) {
                JobStatus.SUCCEEDED -> throw BadRequestException("Стол уже проверен нарезкой")
                JobStatus.FAILED -> Unit
                else -> throw BadRequestException("Проверка нарезкой уже идёт")
            }
        }
        val params = parent.packParams
        val state = parent.directory.resolve(JobArtifacts.PACKED_STATE)
        if (parent.kind != JobKind.PACK || parent.current.status != JobStatus.SUCCEEDED ||
            params == null || parent.packResult?.finalSliced != false || !Files.isRegularFile(state)
        ) {
            throw BadRequestException("Нарезать можно только успешную упаковку без полной нарезки")
        }

        val jobId = UUID.randomUUID().toString()
        val directory = properties.jobsRootPath.resolve(jobId)
        val job = Job(id = jobId, kind = JobKind.PACK, directory = directory, createdAt = OffsetDateTime.now())
        job.parentJobId = parent.id
        // Профиль внутри каталога родителя (разовый стол, локальная копия) переезжает вместе с ним.
        val profilePath = if (params.profilePath.startsWith(parent.directory)) directory.resolve(params.profilePath.fileName) else params.profilePath
        submitter.submit(
            job,
            prepare = {
                for (name in listOf(StlInputs.INPUT_3MF, StlInputs.INPUT_STL, JobArtifacts.PROFILE_FILE)) {
                    val source = parent.directory.resolve(name)
                    if (Files.isRegularFile(source)) Files.copy(source, directory.resolve(name))
                }
                Files.copy(state, directory.resolve(JobArtifacts.FINISH_STATE))
                // STL-вход: проект уже собран родителем (input.3mf) — конвертация не повторяется.
                val sourceCommand = if (Files.isRegularFile(directory.resolve(StlInputs.INPUT_3MF))) null else params.sourceCommand
                job.packParams = params.copy(
                    profilePath = profilePath,
                    sourceCommand = sourceCommand,
                    finalSlice = true,
                    finishState = Path.of(JobArtifacts.FINISH_STATE),
                )
                job.originalName = parent.originalName
                job.profileDto = parent.profileDto
            },
            start = {
                jobRunner.runPack(job, job.packParams!!)
                // Ссылка ставится после успешной постановки; прежняя (упавшая) проверка убирается вместе с каталогом.
                parent.checkJobId?.let { store.remove(it) }?.directory?.toFile()?.deleteRecursively()
                parent.checkJobId = job.id
            },
        )
        log.info("Создана проверка нарезкой {} упаковки {}", jobId, packJobId)
        return JobCreatedDto(parent.id)
    }

    /** Отмена идущей проверки: процесс убивается, каталог и запись удаляются — у упаковки проверки снова нет. */
    fun cancel(packJobId: String) {
        val parent = requireParent(packJobId)
        val child = parent.checkJobId?.let { store.get(it) } ?: return
        if (child.current.status != JobStatus.QUEUED && child.current.status != JobStatus.RUNNING) return
        store.remove(child.id)
        child.cancel()?.takeIf { it.isAlive }?.let(ProcessKiller::kill)
        child.directory.toFile().deleteRecursively()
        parent.checkJobId = null
    }

    private fun requireParent(packJobId: String): Job =
        store.get(packJobId)?.takeIf { it.parentJobId == null } ?: throw BadRequestException("Упаковка «$packJobId» не найдена")
}
