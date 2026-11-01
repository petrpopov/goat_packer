package com.goatpacker.web.service.local

import com.fasterxml.jackson.databind.ObjectMapper
import com.goatpacker.web.dto.ArtifactDto
import com.goatpacker.web.dto.CheckStatusDto
import com.goatpacker.web.dto.JobStageDto
import com.goatpacker.web.dto.JobStatusDto
import com.goatpacker.web.dto.PackInfoDto
import com.goatpacker.web.dto.toDto
import com.goatpacker.web.exception.BadRequestException
import com.goatpacker.web.exception.ResourceNotFoundException
import com.goatpacker.web.model.Job
import com.goatpacker.web.model.JobKind
import com.goatpacker.web.model.JobStatus
import com.goatpacker.web.repository.JobStore
import com.goatpacker.web.service.StlInputs
import com.goatpacker.web.service.runner.JobArtifacts
import com.goatpacker.web.util.ProcessKiller
import org.slf4j.LoggerFactory
import org.springframework.context.annotation.Profile
import org.springframework.http.MediaType
import org.springframework.stereotype.Service
import java.nio.charset.StandardCharsets
import java.nio.file.Files
import java.nio.file.Path

/** Файл на диске для скачивания: путь, имя в `Content-Disposition` и тип содержимого. */
class LocalDownload(val path: Path, val filename: String, val mediaType: MediaType)

/** Чтение статуса джоба, выдача артефактов, отмена и удаление (локальный режим). */
@Service
@Profile("!server")
class JobService(private val store: JobStore) {
    private val log = LoggerFactory.getLogger(javaClass)

    /** Артефакты в статусе; [owner] — id в ссылке, [prefix] — путь скачивания (у проверки: `check/artifacts`, id ребёнка не раскрывается). */
    private fun artifactDtos(job: Job, owner: String, prefix: String): List<ArtifactDto> {
        val files = when (job.current.status) {
            JobStatus.SUCCEEDED -> JobArtifacts.listExisting(job.kind, job.directory)
            JobStatus.FAILED -> JobArtifacts.resolve(job.kind, job.directory, JobArtifacts.LOG_FILE)
                ?.let { listOf(JobArtifacts.LOG_FILE to it) }
                .orEmpty()
            else -> emptyList()
        }
        return files.map { (name, path) -> ArtifactDto(name, sizeOf(path), "/api/jobs/$owner/$prefix/$name") }
    }

    private fun sizeOf(path: Path): Long = try {
        Files.size(path)
    } catch (_: Exception) {
        0
    }

    private fun packInfo(job: Job): PackInfoDto? = job.packResult?.toDto()

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
            stage = snap.stage?.let { JobStageDto.of(it) },
            pack = packInfo(child),
            artifacts = artifactDtos(child, parent.id, "check/artifacts"),
        )
    }

    fun status(id: String): JobStatusDto {
        val job = requireJob(id)
        val snap = job.current
        val isPack = job.kind == JobKind.PACK

        return JobStatusDto(
            jobId = job.id,
            kind = job.kind.jsonValue(),
            status = snap.status.name,
            phase = snap.phase.jsonValue(),
            progressPercent = snap.progressPercent,
            phaseMessage = snap.phaseMessage,
            // Упаковке этап отдаётся в любом статусе: фронт показывает пройденные этапы с длительностями и после завершения.
            stage = snap.stage?.takeIf { isPack || snap.status == JobStatus.RUNNING }?.let { JobStageDto.of(it) },
            // Не только в RUNNING (в отличие от stage): итоговой сцене стола раскладка нужна после успеха.
            // Раскладка проверки (полная нарезка могла сдвинуть объекты) важнее раскладки упаковки, как только она есть.
            layout = (checkOf(job)?.current?.layout ?: snap.layout)?.let { runCatching { LAYOUT_MAPPER.readTree(it) }.getOrNull() },
            error = snap.error,
            originalName = job.originalName.takeIf { isPack },
            profile = job.profileDto.takeIf { isPack },
            params = job.packParams?.takeIf { isPack }?.toDto(),
            createdAt = job.createdAt,
            finishedAt = snap.finishedAt,
            artifacts = artifactDtos(job, job.id, "artifacts"),
            check = if (isPack) checkDto(job) else null,
            pack = packInfo(job),
            schedule = job.scheduleResult?.toDto(),
            calibration = job.calibrationResult?.toDto(),
        )
    }

    fun artifact(id: String, name: String): LocalDownload = artifactOf(requireJob(id), id, name)

    /** Артефакт проверки нарезкой — по id упаковки. */
    fun checkArtifact(id: String, name: String): LocalDownload =
        artifactOf(checkOf(requireJob(id)) ?: throw ResourceNotFoundException("Проверка нарезкой у джобы $id не найдена"), id, name)

    private fun artifactOf(job: Job, id: String, name: String): LocalDownload {
        if (name != JobArtifacts.LOG_FILE && job.current.status != JobStatus.SUCCEEDED) {
            throw ResourceNotFoundException("Артефакт ещё не готов")
        }
        val path = JobArtifacts.resolve(job.kind, job.directory, name)
            ?: throw ResourceNotFoundException("Артефакт «$name» у джобы $id не найден")
        val extension = name.substringAfterLast('.', "").lowercase()

        return LocalDownload(path, name, MEDIA_TYPES[extension] ?: MediaType.APPLICATION_OCTET_STREAM)
    }

    /** Загруженный вход pack-джоба (input.3mf/input.stl) — те же байты, что получил пакер. */
    fun input(id: String): LocalDownload {
        val job = requireJob(id)
        if (job.kind != JobKind.PACK) throw BadRequestException("Вход есть только у упаковки")
        // Для STL-джоба input.3mf — проект, собранный source_project, но input.stl есть всегда и проверяется первым.
        val path = listOf(StlInputs.INPUT_STL, StlInputs.INPUT_3MF).map { job.directory.resolve(it) }.firstOrNull { Files.isRegularFile(it) }
            ?: throw ResourceNotFoundException("Входной файл джобы $id не найден")

        return LocalDownload(path, job.originalName ?: path.fileName.toString(), MediaType.APPLICATION_OCTET_STREAM)
    }

    /** Отмена и удаление: процесс убивается, каталог стирается; проверка нарезкой живёт и умирает с упаковкой. */
    fun delete(id: String) {
        val job = store.get(id)?.takeIf { it.parentJobId == null }?.also { store.remove(id) }
            ?: throw ResourceNotFoundException("Джоба $id не найдена")
        job.checkJobId?.let { store.remove(it) }?.let { child ->
            child.cancel()?.takeIf { it.isAlive }?.let(ProcessKiller::kill)
            child.directory.toFile().deleteRecursively()
        }

        job.cancel()?.takeIf { it.isAlive }?.let { process ->
            log.info("Джоба {} ещё выполняется — убиваю процесс", id)
            ProcessKiller.kill(process)
        }
        try {
            if (Files.exists(job.directory)) job.directory.toFile().deleteRecursively()
        } catch (ex: Exception) {
            log.warn("Не удалось удалить каталог джоба {}: {}", job.directory, ex.message)
        }
        log.info("Джоба {} ({}) удалена", id, job.kind)
    }

    private fun requireJob(id: String): Job =
        store.get(id)?.takeIf { it.parentJobId == null } ?: throw ResourceNotFoundException("Джоба $id не найдена")

    private companion object {
        val LAYOUT_MAPPER = ObjectMapper()

        val MEDIA_TYPES = mapOf(
            "3mf" to MediaType.APPLICATION_OCTET_STREAM,
            "csv" to MediaType.parseMediaType("text/csv"),
            "xlsx" to MediaType.parseMediaType("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"),
            "json" to MediaType.APPLICATION_JSON,
            "txt" to MediaType(MediaType.TEXT_PLAIN, StandardCharsets.UTF_8),
            "md" to MediaType(MediaType.parseMediaType("text/markdown"), StandardCharsets.UTF_8),
        )
    }
}
