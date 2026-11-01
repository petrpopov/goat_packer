package com.goatpacker.web.entity

import com.goatpacker.web.model.JobKind
import com.goatpacker.web.model.JobPhase
import jakarta.persistence.Column
import jakarta.persistence.Entity
import jakarta.persistence.EnumType
import jakarta.persistence.Enumerated
import jakarta.persistence.Id
import jakarta.persistence.Table
import jakarta.persistence.Version
import java.time.Instant
import java.time.LocalDate

private const val EMPTY_JSON = "{}"

/** Строка очереди задач; у проверки нарезкой есть [parentJobId], у её упаковки — [checkJobId]. */
@Entity
@Table(name = "queue_jobs")
class QueueJobEntity(
    @Id var id: String = newId(),
    var ownerId: String = "",
    @Enumerated(EnumType.STRING) var kind: JobKind = JobKind.PACK,
    @Enumerated(EnumType.STRING) var status: QueueStatus = QueueStatus.QUEUED,
    var inputBlobId: String = "",
    var originalName: String = "",
    var profileId: String = "",
    @Column(columnDefinition = "text") var profileJson: String = EMPTY_JSON,
    @Column(columnDefinition = "text") var paramsJson: String = EMPTY_JSON,
    var requestKey: String = "",
    var requestHash: String = "",
    var readTokenHash: String? = null,
    var phase: String = JobPhase.UPLOAD.json,
    var progress: Int = 0,
    var message: String? = null,
    var error: String? = null,
    /** Текущий этап пакера (JSON [JobStage]); null — этапа нет (V3). */
    @Column(columnDefinition = "text") var stageJson: String? = null,
    /** Последняя раскладка пакера (JSON layout-события); null — раскладки нет (V4). */
    @Column(columnDefinition = "text") var layoutJson: String? = null,
    @Column(columnDefinition = "text") var resultJson: String? = null,
    var createdAt: Instant = Instant.now(),
    var startedAt: Instant? = null,
    var finishedAt: Instant? = null,
    var expiresAt: Instant? = null,
    var workerId: String? = null,
    var fence: Long = 0,
    var leaseUntil: Instant? = null,
    var deadlineAt: Instant? = null,
    var cancelRequested: Boolean = false,
    var reservedBytes: Long = 0,
    var attemptCount: Int = 0,
    var availableAt: Instant = Instant.now(),
    var computeReservedSeconds: Long = 0,
    var budgetDay: LocalDate? = null,
    var runnerImage: String? = null,
    /** Проверка нарезкой (V6): у дочерней строки — родитель (в списке она скрыта), у родителя — последняя проверка. */
    var parentJobId: String? = null,
    var checkJobId: String? = null,
    @Version var version: Long = 0,
)
