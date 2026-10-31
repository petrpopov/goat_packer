package com.goatpacker.web.job

import com.fasterxml.jackson.annotation.JsonValue
import java.nio.file.Path
import java.time.OffsetDateTime
import java.util.concurrent.atomic.AtomicReference

/** Тип джоба: упаковка 3MF, построение расписания или калибровка времени смены AMS. */
enum class JobKind {
    PACK,
    SCHEDULE,
    CALIBRATION;

    @JsonValue
    fun jsonValue(): String = name.lowercase()
}

/** Жизненный цикл джоба. */
enum class JobStatus {
    QUEUED,
    RUNNING,
    SUCCEEDED,
    FAILED
}

/**
 * Фаза выполнения. Значения — ровно те, что уходят фронту:
 * upload|probe|packing|assembling|slicing|repacking|benchmark|optimizing|writing|cleanup.
 */
enum class JobPhase(val json: String) {
    UPLOAD("upload"),
    PROBE("probe"),
    PACKING("packing"),
    ASSEMBLING("assembling"),
    SLICING("slicing"),
    REPACKING("repacking"),
    BENCHMARK("benchmark"),
    OPTIMIZING("optimizing"),
    WRITING("writing"),
    CLEANUP("cleanup");

    @JsonValue
    fun jsonValue(): String = json

    companion object {
        fun fromJson(raw: String): JobPhase? = entries.firstOrNull { it.json == raw }
    }
}

/**
 * Текущий этап colored_3mf_packer (stage-события протокола прогресса). Хранится как есть в
 * снимке (локальный режим) или JSON-ом в queue_jobs.stage_json (серверный); прошедшее время
 * этапа считается при выдаче статуса.
 */
data class JobStage(
    /** probe_slice|packing|assembling|final_slice|gcode_check|repacking|window_slice|writing. */
    val id: String,
    /** Начало этапа, epoch ms (часы исполнителя). */
    val startedAtMs: Long,
    /** `expected_s` пакера; null — оценки нет (оставшееся время не показывается). */
    val expectedSeconds: Double? = null,
    val objects: Int? = null,
    /** Номер захода в этот этап (нарезки под окно времени повторяются). */
    val attempt: Int = 1,
    /** Пройденные этапы в порядке первого захода, включая текущий. */
    val visited: List<String> = emptyList(),
    /** Последняя человекочитаемая (английская) строка пакера — вторичная подробность, не JSON. */
    val detail: String? = null,
)

/** Неизменяемый срез статусной сводки джоба — обновляется атомарно, гонок нет. */
data class JobSnapshot(
    val status: JobStatus,
    val phase: JobPhase,
    val progressPercent: Int,
    val phaseMessage: String?,
    val error: String?,
    val finishedAt: OffsetDateTime?,
    val stage: JobStage? = null,
)

/** Результат pack-джоба, распарсенный из verification.json после успеха. */
data class PackJobResult(
    val objectCount: Int,
    val profileId: String,
    val clearanceMm: Double,
    /** Оценка времени из блока `time` verification.json; null — не измерено. */
    val time: PackTimeResult? = null,
    /**
     * `supports_detected` отчёта: у модели есть поддержки за пределами силуэта, поворот
     * заблокирован. null — старый пакер без этого поля.
     */
    val supportsDetected: Boolean? = null,
    /** `support_overlap_mm` отчёта: допустимое перекрытие поддержек, мм; null — без ограничений. */
    val supportOverlapMm: Double? = null,
)

/**
 * Результат calibration-джоба: calibration.json бинарника ams_calibration + текст
 * instructions.md (небольшой — отдаётся прямо в статусе, чтобы фронт не тянул файл отдельно).
 */
data class CalibrationJobResult(
    val profileId: String,
    val variant: String,
    val changesExpected: Int,
    val changesSliced: Int,
    val estimateASeconds: Double,
    val estimateBSeconds: Double,
    val slicerChangeSeconds: Double,
    val instructions: String,
)

/** Сводка блока `time` отчёта упаковщика (секунды; см. [com.goatpacker.web.dto.PackTimeDto]). */
data class PackTimeResult(
    val windowHours: Double?,
    val marginPct: Double,
    val realChangeSeconds: Double?,
    val chosenCount: Int,
    val fullCount: Int,
    val correctedSeconds: Double,
)

/** Одна работа в расписании (из report.json → variable_batch_schedule.jobs). */
data class ScheduleJobRecord(
    val sequence: Int,
    val batchSize: Int,
    val startSeconds: Long,
    val endSeconds: Long,
    val slicerSeconds: Long,
    val preparationSeconds: Long,
    val filamentGrams: Double,
    val cumulativeObjects: Int,
    val overnight: Boolean,
)

/** Одна пластина мультипластинного 3MF (уникальный batch_size в порядке появления). */
data class PlateRecord(
    val label: String,
    val objectCount: Int,
)

/** Результат schedule-джоба, распарсенный из report.json после успеха. */
data class ScheduleJobResult(
    val printer: String,
    val totalObjects: Int,
    val totalFilamentGrams: Double,
    val completionClock: String,
    val jobs: List<ScheduleJobRecord>,
    val plates: List<PlateRecord>,
)

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

    @Volatile
    var scheduleResult: ScheduleJobResult? = null

    @Volatile
    var calibrationResult: CalibrationJobResult? = null

    val current: JobSnapshot
        get() = snapshot.get()

    var runtimeFence: Long = 0
    var runtimeImage: String? = null
    var runtimeTimeoutSeconds: Long = 1800
    var onUpdate: ((JobSnapshot) -> Unit)? = null

    fun update(transform: (JobSnapshot) -> JobSnapshot) {
        val next = snapshot.updateAndGet { if (cancelled) it else transform(it) }
        onUpdate?.invoke(next)
    }
}
