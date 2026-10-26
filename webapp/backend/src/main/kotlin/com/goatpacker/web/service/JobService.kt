package com.goatpacker.web.service

import com.goatpacker.web.dto.ArtifactDto
import com.goatpacker.web.dto.JobStatusDto
import com.goatpacker.web.dto.PackInfoDto
import com.goatpacker.web.dto.PackTimeDto
import com.goatpacker.web.dto.PlateDto
import com.goatpacker.web.dto.ScheduleInfoDto
import com.goatpacker.web.dto.ScheduleJobDto
import com.goatpacker.web.exception.ResourceNotFoundException
import com.goatpacker.web.job.Job
import com.goatpacker.web.job.JobArtifacts
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

        private val MEDIA_TYPES = mapOf(
            "3mf" to MediaType.APPLICATION_OCTET_STREAM,
            "csv" to MediaType.parseMediaType("text/csv"),
            "xlsx" to MediaType.parseMediaType("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"),
            "json" to MediaType.APPLICATION_JSON,
            "txt" to MediaType(MediaType.TEXT_PLAIN, StandardCharsets.UTF_8),
        )
    }

    fun status(id: String): JobStatusDto {
        val job = requireJob(id)
        val snap = job.current

        val artifacts = when (snap.status) {
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
                downloadUrl = "/api/jobs/${job.id}/artifacts/$name",
            )
        }

        return JobStatusDto(
            jobId = job.id,
            kind = job.kind.jsonValue(),
            status = snap.status.name,
            phase = snap.phase.jsonValue(),
            progressPercent = snap.progressPercent,
            phaseMessage = snap.phaseMessage,
            error = snap.error,
            createdAt = job.createdAt,
            finishedAt = snap.finishedAt,
            artifacts = artifacts,
            pack = job.packResult?.let {
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
                )
            },
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
        )
    }

    fun artifact(id: String, name: String): ResponseEntity<FileSystemResource> {
        val job = requireJob(id)
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

    fun delete(id: String): ResponseEntity<Void> {
        val job = store.remove(id)
            ?: throw ResourceNotFoundException("Джоба $id не найдена")

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
        store.get(id) ?: throw ResourceNotFoundException("Джоба $id не найдена")
}
