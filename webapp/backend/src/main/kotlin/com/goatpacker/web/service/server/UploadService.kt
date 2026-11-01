package com.goatpacker.web.service.server

import com.fasterxml.jackson.databind.ObjectMapper
import com.goatpacker.web.config.PlatformProperties
import com.goatpacker.web.dto.UploadDto
import com.goatpacker.web.dto.UploadIntent
import com.goatpacker.web.entity.BlobEntity
import com.goatpacker.web.entity.PrincipalKind
import com.goatpacker.web.entity.UploadEntity
import com.goatpacker.web.entity.UploadState
import com.goatpacker.web.exception.ApiException
import com.goatpacker.web.exception.ErrorCode
import com.goatpacker.web.exception.accessBlocked
import com.goatpacker.web.exception.badRequest
import com.goatpacker.web.exception.notFound
import com.goatpacker.web.exception.quotaExceeded
import com.goatpacker.web.exception.underMaintenance
import com.goatpacker.web.model.Inspection
import com.goatpacker.web.model.UploadKind
import com.goatpacker.web.repository.BlobRepository
import com.goatpacker.web.repository.ObjectGcRepository
import com.goatpacker.web.repository.OperationalStateRepository
import com.goatpacker.web.repository.PlatformRepository
import com.goatpacker.web.repository.PrincipalRepository
import com.goatpacker.web.repository.UploadRepository
import com.goatpacker.web.service.StlInputs
import com.goatpacker.web.util.safeName
import org.springframework.context.annotation.Profile
import org.springframework.http.HttpStatus
import org.springframework.stereotype.Service
import org.springframework.transaction.annotation.Transactional
import java.time.Instant

/** Жизненный цикл загрузки в БД: резерв места → приём байт → готово. Сами байты принимает [UploadReceiver]. */
@Service
@Profile("server")
class UploadService(
    private val uploads: UploadRepository,
    private val blobs: BlobRepository,
    private val owners: PrincipalRepository,
    private val platform: PlatformRepository,
    private val quotas: StorageQuotaService,
    private val gc: ObjectGcRepository,
    private val operationalState: OperationalStateRepository,
    private val challenges: ChallengeService,
    private val props: PlatformProperties,
    private val mapper: ObjectMapper,
) {
    /** Резервирует место под файл: после этого клиент может передать байты. */
    @Transactional
    fun reserve(ownerId: String, intent: UploadIntent): UploadDto {
        if (!StlInputs.isAcceptedUpload(intent.name)) badRequest(StlInputs.UPLOAD_EXTENSIONS_MESSAGE)
        if (intent.sizeBytes !in 1..props.maxUploadBytes) {
            throw ApiException(HttpStatus.PAYLOAD_TOO_LARGE, ErrorCode.FILE_TOO_LARGE, "Допустимо до ${props.maxUploadBytes / BYTES_PER_MIB} МиБ")
        }
        platform.lockForUpdate()
        val owner = owners.findByIdForUpdate(ownerId) ?: notFound()
        if (operationalState.isMaintenance()) underMaintenance("Сервис временно на обслуживании")
        challenges.consume(ownerId, intent.challengeId, intent.nonce)
        quotas.reserve(owner, intent.sizeBytes)
        if (uploads.countByStateIn(UploadState.IN_PROGRESS) >= props.maxActiveUploads) quotaExceeded("Все места загрузки заняты")
        if (uploads.findAllByOwnerId(ownerId).any { it.state in UploadState.IN_PROGRESS }) {
            quotaExceeded("У вас уже есть активная загрузка")
        }
        return uploads.save(UploadEntity(ownerId = ownerId, originalName = safeName(intent.name), expectedBytes = intent.sizeBytes)).toDto()
    }

    /** Переводит загрузку в приём байт и страхует объект от потери: если приём не завершится, объект уберёт GC. */
    @Transactional
    fun beginReceiving(id: String, ownerId: String, contentLength: Long): UploadEntity {
        platform.lockForUpdate()
        val upload = findOwned(id, ownerId)
        if (upload.state != UploadState.UPLOADING || upload.expiresAt?.isBefore(Instant.now()) == true) {
            badRequest("Загрузка истекла или уже началась")
        }
        if (contentLength != upload.expectedBytes) badRequest("Размер загрузки не совпадает с зарезервированным")
        upload.state = UploadState.RECEIVING
        gc.stage(objectKey(id), upload.expectedBytes)
        return upload
    }

    /** Объект залит и разобран: загрузка готова к использованию в задаче. Одинаковые байты делят один блоб. */
    @Transactional
    fun complete(id: String, ownerId: String, received: ReceivedFile): UploadDto {
        platform.lockForUpdate()
        val owner = owners.findByIdForUpdate(ownerId) ?: notFound()
        if (owner.blocked) accessBlocked()
        val upload = findOwned(id, ownerId)
        if (upload.state != UploadState.RECEIVING || upload.expiresAt?.isBefore(Instant.now()) == true) {
            badRequest("Загрузка больше не активна")
        }
        check(gc.lockLiveStaging(received.objectKey)) { "Upload staging expired" }
        val existing = blobs.findBySha256AndSizeBytes(received.sha256, upload.expectedBytes)
        val blob = existing ?: blobs.save(BlobEntity(sha256 = received.sha256, sizeBytes = upload.expectedBytes, objectKey = received.objectKey))
        if (existing == null) gc.remove(received.objectKey)
        upload.blobId = blob.id
        upload.state = UploadState.READY
        upload.printerModel = received.inspection.printerModel
        upload.profileId = received.profileId
        upload.projectBedJson = received.projectBedJson
        upload.expiresAt = if (owner.kind == PrincipalKind.GUEST) Instant.now().plusSeconds(RetentionPolicy.GUEST_CONTENT_TTL_SECONDS) else null
        return upload.toDto()
    }

    /** Приём не удался: незавершённую загрузку убираем; объект остаётся на совести GC (commit мог пройти при потерянном ответе). */
    @Transactional
    fun abort(id: String, ownerId: String) {
        platform.lockForUpdate()
        uploads.findById(id).orElse(null)
            ?.takeIf { it.ownerId == ownerId && it.state != UploadState.READY }
            ?.let(uploads::delete)
    }

    @Transactional
    fun delete(id: String, ownerId: String) {
        platform.lockForUpdate()
        val upload = findOwned(id, ownerId)
        if (upload.state == UploadState.RECEIVING) badRequest("Дождитесь конца загрузки")
        uploads.delete(upload)
    }

    fun list(ownerId: String): List<UploadDto> =
        uploads.findAllByOwnerId(ownerId).sortedByDescending { it.createdAt }.take(MAX_LISTED).map { it.toDto() }

    fun get(id: String, ownerId: String): UploadDto = findOwned(id, ownerId).toDto()

    private fun UploadEntity.toDto() = UploadDto(
        fileId = id,
        originalName = originalName,
        sizeBytes = expectedBytes,
        state = state,
        profileId = profileId,
        printerModel = printerModel,
        projectBed = projectBedJson?.let { runCatching { mapper.readTree(it) }.getOrNull() },
        kind = if (StlInputs.isStl(originalName)) UploadKind.MESH_STL else UploadKind.PROJECT_3MF,
        expiresAt = expiresAt,
    )

    private fun findOwned(id: String, ownerId: String): UploadEntity =
        uploads.findById(id).orElse(null)?.takeIf { it.ownerId == ownerId } ?: notFound()

    companion object {
        private const val BYTES_PER_MIB = 1024 * 1024
        private const val MAX_LISTED = 100

        /** Ключ объекта загрузки в хранилище. */
        fun objectKey(uploadId: String): String = "inputs/$uploadId"
    }
}

/** Принятый и разобранный файл загрузки: всё, что нужно записать в БД. */
data class ReceivedFile(
    val objectKey: String,
    val sha256: String,
    val inspection: Inspection,
    val profileId: String?,
    val projectBedJson: String?,
)
