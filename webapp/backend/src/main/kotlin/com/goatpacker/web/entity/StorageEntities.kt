package com.goatpacker.web.entity

import jakarta.persistence.Column
import jakarta.persistence.Entity
import jakarta.persistence.EnumType
import jakarta.persistence.Enumerated
import jakarta.persistence.Id
import jakarta.persistence.Table
import jakarta.persistence.Version
import java.time.Instant

/** Неизменяемый объект хранилища; одинаковые байты (sha256 + размер) делят один блоб. */
@Entity
@Table(name = "blobs")
class BlobEntity(
    @Id var id: String = newId(),
    var sha256: String = "",
    var sizeBytes: Long = 0,
    var objectKey: String = "",
    var createdAt: Instant = Instant.now(),
)

/** Загруженный пользователем файл, ещё не привязанный к задаче. */
@Entity
@Table(name = "uploads")
class UploadEntity(
    @Id var id: String = newId(),
    var ownerId: String = "",
    var originalName: String = "",
    var expectedBytes: Long = 0,
    @Enumerated(EnumType.STRING) var state: UploadState = UploadState.UPLOADING,
    var blobId: String? = null,
    var printerModel: String? = null,
    var profileId: String? = null,
    @Column(columnDefinition = "text") var projectBedJson: String? = null,
    var createdAt: Instant = Instant.now(),
    var expiresAt: Instant? = Instant.now().plusSeconds(UPLOAD_RESERVATION_SECONDS),
    @Version var version: Long = 0,
) {
    companion object {
        /** Сколько зарезервированная загрузка ждёт начала передачи байт. */
        const val UPLOAD_RESERVATION_SECONDS = 900L
    }
}

/** Файл-результат задачи в хранилище; [debug] — диагностика, живёт недолго и скрыта по умолчанию. */
@Entity
@Table(name = "artifacts")
class ArtifactEntity(
    @Id var id: String = newId(),
    var jobId: String = "",
    var name: String = "",
    var downloadName: String = "",
    var objectKey: String = "",
    var sizeBytes: Long = 0,
    var debug: Boolean = false,
    var createdAt: Instant = Instant.now(),
)
