package com.goatpacker.web.service

import com.goatpacker.web.dto.ArtifactDto
import com.goatpacker.web.dto.CalibrationInfoDto
import com.goatpacker.web.dto.CheckStatusDto
import com.goatpacker.web.dto.JobStatusDto
import com.goatpacker.web.dto.PackInfoDto
import com.goatpacker.web.dto.PackTimeDto
import com.goatpacker.web.dto.PlateDto
import com.goatpacker.web.dto.ScheduleInfoDto
import com.goatpacker.web.dto.ScheduleJobDto
import com.goatpacker.web.exception.ResourceNotFoundException
import com.goatpacker.web.job.Job
import com.goatpacker.web.job.JobArtifacts
import com.goatpacker.web.job.JobKind
import com.goatpacker.web.job.JobStatus
import com.goatpacker.web.job.JobStore
import com.goatpacker.web.job.JobProcessKiller
import org.slf4j.LoggerFactory
import org.springframework.core.io.FileSystemResource
import org.springframework.http.ContentDisposition
import org.springframework.http.HttpHeaders
import org.springframework.http.HttpStatus
import org.springframework.http.MediaType
import org.springframework.http.ResponseEntity
import org.springframework.stereotype.Service
import java.nio.charset.StandardCharsets
import java.nio.file.Files

/** Чтение статуса джоба, выдача артефактов, отмена и удаление. */
@Service
@org.springframework.context.annotation.Profile("!server")
class JobService(
    private val store: JobStore,
) {
    companion object {
        private val log = LoggerFactory.getLogger(JobService::class.java)
        private val layoutMapper = com.fasterxml.jackson.databind.ObjectMapper()

        private val MEDIA_TYPES = mapOf(
            "3mf" to MediaType.APPLICATION_OCTET_STREAM,
            "csv" to MediaType.parseMediaType("text/csv"),
            "xlsx" to MediaType.parseMediaType("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"),
            "json" to MediaType.APPLICATION_JSON,
            "txt" to MediaType(MediaType.TEXT_PLAIN, StandardCharsets.UTF_8),
            "md" to MediaType(MediaType.parseMediaType("text/markdown"), StandardCharsets.UTF_8),
        )
    }

    /** Артефакты в статусе; [owner] — id в ссылке, [prefix] — путь скачивания (у проверки: `check/artifacts`, id ребёнка не раскрывается). */
    private fun artifactDtos(job: Job, owner: String, prefix: String): List<ArtifactDto> {
        val snap = job.current
        return when (snap.status) {
            JobStatus.SUCCEEDED -> JobArtifacts.listExisting(job.kind, job.directory)
            JobStatus.FAILED -> JobArtifacts.resolve(job.kind, job.directory, JobArtifacts.LOG_FILE)
                ?.let { listOf(JobArtifacts.LOG_FILE to it) }
                ?: emptyList()

            else -> emptyList()
        }.map { (name, path) ->
            ArtifactDto(
                name = name,
                sizeBytes = try {
                    Files.size(path)
                } catch (_: Exception) {
                    0
                },
                downloadUrl = "/api/jobs/$owner/$prefix/$name",
            )
        }
    }

    private fun packInfo(job: Job): PackInfoDto? = job.packResult?.let {
        PackInfoDto(
            objectCount = it.objectCount,
            profileId = it.profileId,
            clearanceMm = it.clearanceMm,
            time = it.time?.let { t ->
                PackTimeDto(
                    windowHours = t.windowHours,
                    marginPct = t.marginPct,
                    realChangeSeconds = t.realChangeSeconds,
                    chosenCount = t.chosenCount,
                    fullCount = t.fullCount,
                    correctedSeconds = t.correctedSeconds,
                )
            },
            supportsDetected = it.supportsDetected,
            supportOverlapMm = it.supportOverlapMm,
            finalSliced = it.finalSliced,
        )
    }

    /** Проверка нарезкой упаковки (внутренняя задача); её id наружу не уходит. */
    private fun checkOf(parent: Job): Job? = parent.checkJobId?.let { store.get(it) }?.takeIf { it.parentJobId == parent.id }

    private fun checkDto(parent: Job): CheckStatusDto? = checkOf(parent)?.let { child ->
        val snap = child.current
        CheckStatusDto(
            status = snap.status.name,
            queuePosition = null,
            progressPercent = snap.progressPercent,
            phaseMessage = snap.phaseMessage,
            error = snap.error,
            // Своего времени старта у локальной задачи нет: она стартует сразу, как свободен единственный поток.
            startedAt = child.createdAt.takeIf { snap.status != JobStatus.QUEUED },
            finishedAt = snap.finishedAt,
            stage = snap.stage?.let { com.goatpacker.web.dto.JobStageDto.of(it) },
            pack = packInfo(child),
            artifacts = artifactDtos(child, parent.id, "check/artifacts"),
        )
    }

    fun status(id: String): JobStatusDto {
        val job = requireJob(id)
        val snap = job.current
        val artifacts = artifactDtos(job, job.id, "artifacts")
        val check = if (job.kind == JobKind.PACK) checkDto(job) else null

        return JobStatusDto(
            jobId = job.id,
            kind = job.kind.jsonValue(),
            status = snap.status.name,
            phase = snap.phase.jsonValue(),
            progressPercent = snap.progressPercent,
            phaseMessage = snap.phaseMessage,
            // Упаковке этап отдаётся в любом статусе: фронт показывает пройденные этапы с длительностями и после завершения.
            stage = snap.stage?.takeIf { job.kind == JobKind.PACK || snap.status == JobStatus.RUNNING }?.let { com.goatpacker.web.dto.JobStageDto.of(it) },
            // Не только в RUNNING (в отличие от stage): итоговой сцене стола раскладка нужна после успеха.
            // Раскладка проверки (полная нарезка могла сдвинуть объекты) важнее раскладки упаковки, как только она есть.
            layout = (checkOf(job)?.current?.layout ?: snap.layout)?.let { runCatching { layoutMapper.readTree(it) }.getOrNull() },
            error = snap.error,
            originalName = job.originalName.takeIf { job.kind == JobKind.PACK },
            profile = job.profileDto.takeIf { job.kind == JobKind.PACK },
            params = job.packParams?.takeIf { job.kind == JobKind.PACK }?.toDto(),
            createdAt = job.createdAt,
            finishedAt = snap.finishedAt,
            artifacts = artifacts,
            check = check,
            pack = packInfo(job),
            schedule = job.scheduleResult?.let {
                ScheduleInfoDto(
                    printer = it.printer,
                    totalObjects = it.totalObjects,
                    totalFilamentGrams = it.totalFilamentGrams,
                    completionClock = it.completionClock,
                    jobs = it.jobs.map { record ->
                        ScheduleJobDto(
                            sequence = record.sequence,
                            batchSize = record.batchSize,
                            startClock = ClockFormat.format(record.startSeconds),
                            endClock = ClockFormat.format(record.endSeconds),
                            startSeconds = record.startSeconds,
                            endSeconds = record.endSeconds,
                            slicerSeconds = record.slicerSeconds,
                            preparationSeconds = record.preparationSeconds,
                            filamentGrams = record.filamentGrams,
                            cumulativeObjects = record.cumulativeObjects,
                            overnight = record.overnight,
                        )
                    },
                    plates = it.plates.map { plate -> PlateDto(label = plate.label, objectCount = plate.objectCount) },
                )
            },
            calibration = job.calibrationResult?.let {
                CalibrationInfoDto(
                    profileId = it.profileId,
                    variant = it.variant,
                    changesExpected = it.changesExpected,
                    changesSliced = it.changesSliced,
                    estimateASeconds = it.estimateASeconds,
                    estimateBSeconds = it.estimateBSeconds,
                    slicerChangeSeconds = it.slicerChangeSeconds,
                    instructions = it.instructions,
                )
            },
        )
    }

    fun artifact(id: String, name: String): ResponseEntity<FileSystemResource> = artifactOf(requireJob(id), id, name)

    /** Артефакт проверки нарезкой — по id упаковки. */
    fun checkArtifact(id: String, name: String): ResponseEntity<FileSystemResource> =
        artifactOf(checkOf(requireJob(id)) ?: throw ResourceNotFoundException("Проверка нарезкой у джобы $id не найдена"), id, name)

    private fun artifactOf(job: Job, id: String, name: String): ResponseEntity<FileSystemResource> {
        if (name != JobArtifacts.LOG_FILE && job.current.status != JobStatus.SUCCEEDED) {
            throw ResourceNotFoundException("Артефакт ещё не готов")
        }
        val path = JobArtifacts.resolve(job.kind, job.directory, name)
            ?: throw ResourceNotFoundException("Артефакт «$name» у джобы $id не найден")

        val extension = name.substringAfterLast('.', "").lowercase()
        val headers = HttpHeaders().apply {
            contentDisposition = ContentDisposition.attachment().filename(name, StandardCharsets.UTF_8).build()
            contentType = MEDIA_TYPES[extension] ?: MediaType.APPLICATION_OCTET_STREAM
        }

        return ResponseEntity.ok().headers(headers).body(FileSystemResource(path))
    }

    /** Загруженный вход pack-джоба (input.3mf/input.stl) — те же байты, что получил пакер. */
    fun input(id: String): ResponseEntity<FileSystemResource> {
        val job = requireJob(id)
        if (job.kind != JobKind.PACK) throw com.goatpacker.web.exception.BadRequestException("Вход есть только у упаковки")
        val path = listOf(StlInputs.INPUT_STL, StlInputs.INPUT_3MF).map { job.directory.resolve(it) }.firstOrNull { Files.isRegularFile(it) }
            ?: throw ResourceNotFoundException("Входной файл джобы $id не найден")
        // Для STL-джоба input.3mf — проект, собранный source_project, но input.stl есть всегда и проверяется первым.
        val name = job.originalName ?: path.fileName.toString()
        val headers = HttpHeaders().apply {
            contentDisposition = ContentDisposition.attachment().filename(name, StandardCharsets.UTF_8).build()
            contentType = MediaType.APPLICATION_OCTET_STREAM
        }
        return ResponseEntity.ok().headers(headers).body(FileSystemResource(path))
    }

    fun delete(id: String): ResponseEntity<Void> {
        val job = store.get(id)?.takeIf { it.parentJobId == null }?.also { store.remove(id) }
            ?: throw ResourceNotFoundException("Джоба $id не найдена")
        // Проверка нарезкой живёт и умирает с упаковкой.
        job.checkJobId?.let { store.remove(it) }?.let { child ->
            child.cancel()?.takeIf { it.isAlive }?.let { process -> JobProcessKiller.kill(process) }
            child.directory.toFile().deleteRecursively()
        }

        job.cancel()?.takeIf { it.isAlive }?.let { process ->
            log.info("Джоба {} ещё выполняется — убиваю процесс", id)
            JobProcessKiller.kill(process)
        }
        try {
            if (Files.exists(job.directory)) {
                job.directory.toFile().deleteRecursively()
            }
        } catch (ex: Exception) {
            log.warn("Не удалось удалить каталог джоба {}: {}", job.directory, ex.message)
        }
        log.info("Джоба {} ({}) удалена", id, job.kind)

        return ResponseEntity.status(HttpStatus.NO_CONTENT).build()
    }

    private fun requireJob(id: String): Job =
        store.get(id)?.takeIf { it.parentJobId == null } ?: throw ResourceNotFoundException("Джоба $id не найдена")
}
