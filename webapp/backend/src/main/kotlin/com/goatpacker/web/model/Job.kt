package com.goatpacker.web.model

import com.goatpacker.web.dto.ProfileDto
import java.nio.file.Path
import java.time.OffsetDateTime
import java.util.concurrent.atomic.AtomicReference

/**
 * Джоб в памяти: неизменяемая идентичность + атомарная статусная сводка. Единственный писатель —
 * поток JobRunner, читатели — HTTP-потоки, поэтому AtomicReference достаточно.
 */
class Job(
    val id: String,
    val kind: JobKind,
    val directory: Path,
    val createdAt: OffsetDateTime,
) {
    @Volatile
    var cancelled = false
        private set

    @Synchronized
    fun cancel(): Process? {
        cancelled = true
        return process
    }

    val snapshot = AtomicReference(
        JobSnapshot(
            status = JobStatus.QUEUED,
            phase = JobPhase.UPLOAD,
            progressPercent = 0,
            phaseMessage = null,
            error = null,
            finishedAt = null,
        ),
    )

    /** Живой процесс бинарника; выставляется JobRunner'ом, читается при DELETE. */
    @Volatile
    var process: Process? = null

    @Volatile
    var packResult: PackJobResult? = null

    /** Параметры pack-джоба (локальный режим): доводка «Нарезать стол» повторяет их. */
    @Volatile
    var packParams: PackParams? = null

    /** Имя загруженного файла и профиль для статуса pack-джоба (локальный режим); доводка копирует их у родителя. */
    @Volatile
    var originalName: String? = null

    @Volatile
    var profileDto: ProfileDto? = null

    /** Проверка нарезкой (локальный режим): у упаковки — id её внутренней задачи, у внутренней — id упаковки. Наружу не отдаётся. */
    @Volatile
    var checkJobId: String? = null

    @Volatile
    var parentJobId: String? = null

    @Volatile
    var scheduleResult: ScheduleJobResult? = null

    @Volatile
    var calibrationResult: CalibrationJobResult? = null

    val current: JobSnapshot
        get() = snapshot.get()

    var runtimeFence: Long = 0
    var runtimeImage: String? = null
    var runtimeTimeoutSeconds: Long = DEFAULT_RUNTIME_TIMEOUT_SECONDS
    var onUpdate: ((JobSnapshot) -> Unit)? = null

    fun update(transform: (JobSnapshot) -> JobSnapshot) {
        val next = snapshot.updateAndGet { if (cancelled) it else transform(it) }
        onUpdate?.invoke(next)
    }

    private companion object {
        /** Лимит выполнения, пока воркер не выставил реальный дедлайн задачи. */
        const val DEFAULT_RUNTIME_TIMEOUT_SECONDS = 1800L
    }
}
