package com.goatpacker.web.service.server

import com.fasterxml.jackson.databind.ObjectMapper
import com.goatpacker.web.config.PlatformProperties
import com.goatpacker.web.dto.UploadDto
import com.goatpacker.web.exception.badRequest
import com.goatpacker.web.model.Inspection
import com.goatpacker.web.repository.ObjectStore
import com.goatpacker.web.security.toHex
import com.goatpacker.web.service.ProfileService
import com.goatpacker.web.service.Project3mf
import com.goatpacker.web.service.StlInputs
import com.goatpacker.web.util.STREAM_BUFFER_BYTES
import org.springframework.context.annotation.Profile
import org.springframework.stereotype.Service
import java.io.InputStream
import java.nio.file.Files
import java.nio.file.Path
import java.security.MessageDigest
import java.util.concurrent.TimeUnit

/**
 * Приём байт загрузки: потоком на диск с контролем размера и времени, разбор, заливка в хранилище. БД трогают
 * только короткие транзакции [UploadService] — на время передачи ни одна не держится.
 */
@Service
@Profile("server")
class UploadReceiver(
    private val uploads: UploadService,
    private val store: ObjectStore,
    private val inspector: Project3mf,
    private val profiles: ProfileService,
    private val props: PlatformProperties,
    private val mapper: ObjectMapper,
) {
    fun receive(id: String, ownerId: String, contentLength: Long, body: InputStream): UploadDto {
        val upload = uploads.beginReceiving(id, ownerId, contentLength)
        val file = Files.createTempFile("goat-upload-", ".3mf")
        val key = UploadService.objectKey(id)
        try {
            val sha256 = copyWithLimits(body, file, upload.expectedBytes)
            val inspection = inspect(file, upload.originalName)
            val detection = profiles.detect(inspection)
            store.put(key, file)
            val received = ReceivedFile(key, sha256, inspection, detection.profileId, detection.projectBed?.let(mapper::writeValueAsString))
            return uploads.complete(id, ownerId, received)
        } catch (ex: Exception) {
            // Only durable GC may delete: finish may have committed despite a lost DB reply.
            runCatching { uploads.abort(id, ownerId) }
            throw ex
        } finally {
            Files.deleteIfExists(file)
        }
    }

    /** Копирует поток в [target], не превышая [expectedBytes] и срок [PlatformProperties.uploadTimeoutSeconds]; возвращает sha256. */
    private fun copyWithLimits(body: InputStream, target: Path, expectedBytes: Long): String {
        val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(props.uploadTimeoutSeconds)
        val digest = MessageDigest.getInstance("SHA-256")
        var copied = 0L
        body.use { input ->
            Files.newOutputStream(target).use { output ->
                val buffer = ByteArray(STREAM_BUFFER_BYTES)
                while (true) {
                    if (System.nanoTime() > deadline) badRequest("Время загрузки превышено")
                    val read = input.read(buffer)
                    if (read < 0) break
                    copied += read
                    if (copied > expectedBytes) badRequest("Размер файла превышен")
                    digest.update(buffer, 0, read)
                    output.write(buffer, 0, read)
                }
            }
        }
        if (copied != expectedBytes) badRequest("Загрузка неполная")
        return digest.digest().toHex()
    }

    /** STL — сетка без настроек слайсера: не разбирается, принтер выберет пользователь при создании задачи. */
    private fun inspect(file: Path, originalName: String): Inspection {
        if (StlInputs.isStl(originalName)) {
            StlInputs.requireValidStl(file)
            return Inspection(hasModel = false, printerModel = null)
        }
        return inspector.inspect(file).also(inspector::requireModel)
    }
}
