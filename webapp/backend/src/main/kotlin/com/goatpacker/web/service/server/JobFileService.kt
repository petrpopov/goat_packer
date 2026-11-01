package com.goatpacker.web.service.server

import com.goatpacker.web.dto.DownloadLinkDto
import com.goatpacker.web.entity.QueueJobEntity
import com.goatpacker.web.exception.badRequest
import com.goatpacker.web.exception.notFound
import com.goatpacker.web.model.JobKind
import com.goatpacker.web.repository.BlobRepository
import com.goatpacker.web.repository.ObjectStore
import org.springframework.context.annotation.Profile
import org.springframework.stereotype.Service
import java.nio.file.Files
import java.time.Instant

/** Содержимое файла для ответа: имя в `Content-Disposition` и байты. */
class FileDownload(val filename: String, val bytes: ByteArray)

/**
 * Скачивание результатов и входа задачи: подписанная ссылка хранилища (если оно умеет) или запасной маршрут
 * API, который отдаёт байты сам. Доступ проверяется так же, как при чтении статуса.
 */
@Service
@Profile("server")
class JobFileService(
    private val access: JobAccessService,
    private val blobs: BlobRepository,
    private val store: ObjectStore,
) {
    fun artifactLink(viewerId: String, jobId: String, artifactId: String, token: String?): DownloadLinkDto {
        val job = access.findReadable(jobId, viewerId, token)
        val artifact = access.findArtifact(job, artifactId) ?: notFound()
        val url = store.signedGet(artifact.objectKey, artifact.downloadName, linkSeconds(job))
            ?: "/api/v2/jobs/$jobId/artifacts/$artifactId/file"
        return DownloadLinkDto(url, artifact.downloadName)
    }

    fun artifactFile(viewerId: String, jobId: String, artifactId: String): FileDownload {
        val artifact = access.findArtifact(access.findReadable(jobId, viewerId, null), artifactId) ?: notFound()
        return read(artifact.objectKey, artifact.downloadName)
    }

    fun inputLink(viewerId: String, jobId: String, token: String?): DownloadLinkDto {
        val job = packJob(viewerId, jobId, token)
        val blob = blobs.findById(job.inputBlobId).orElse(null) ?: notFound()
        val url = store.signedGet(blob.objectKey, job.originalName, linkSeconds(job)) ?: "/api/v2/jobs/$jobId/input/file"
        return DownloadLinkDto(url, job.originalName)
    }

    fun inputFile(viewerId: String, jobId: String): FileDownload {
        val job = packJob(viewerId, jobId, null)
        val blob = blobs.findById(job.inputBlobId).orElse(null) ?: notFound()
        return read(blob.objectKey, job.originalName)
    }

    /** Вход есть только у упаковки: у расписания и калибровки это внутренний промежуточный файл. */
    private fun packJob(viewerId: String, jobId: String, token: String?): QueueJobEntity {
        val job = access.findReadable(jobId, viewerId, token)
        if (job.kind != JobKind.PACK) badRequest("Вход есть только у упаковки")
        return job
    }

    /** Ссылка живёт не дольше срока хранения задачи и не дольше [MAX_LINK_SECONDS]. */
    private fun linkSeconds(job: QueueJobEntity): Long =
        job.expiresAt?.let { (it.epochSecond - Instant.now().epochSecond).coerceIn(1, MAX_LINK_SECONDS) } ?: MAX_LINK_SECONDS

    private fun read(objectKey: String, filename: String): FileDownload {
        val temporary = Files.createTempFile("goat-download-", ".bin")
        try {
            store.get(objectKey, temporary)
            return FileDownload(filename, Files.readAllBytes(temporary))
        } finally {
            Files.deleteIfExists(temporary)
        }
    }

    private companion object {
        const val MAX_LINK_SECONDS = 300L
    }
}
