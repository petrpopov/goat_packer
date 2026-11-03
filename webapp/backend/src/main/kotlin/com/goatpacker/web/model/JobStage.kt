package com.goatpacker.web.model

import java.time.OffsetDateTime

/**
 * Текущий этап colored_3mf_packer (stage-события протокола прогресса). Хранится как есть в
 * снимке (локальный режим) или JSON-ом в queue_jobs.stage_json (серверный); прошедшее время
 * этапа считается при выдаче статуса.
 */
data class JobStage(
    /** probe_slice|packing|sample_slice|assembling|final_slice|gcode_check|repacking|window_slice|writing. */
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
    /** Все заходы в этапы по порядку (для списка пройденных этапов с длительностями после завершения). */
    val history: List<StageSpan> = emptyList(),
    /**
     * Живая подпись шага внутри этапа из `detail` stage-события («поворот 180° · прогон 3 (до 8)»,
     * «вариант зазора 2 из 4 · сетка 0,25 мм»); null — пакер шагов не присылает.
     */
    val label: String? = null,
    /** Этапы, которые пакер наметил на этот запуск (`plan`-событие); пусто — старый пакер, план не известен. */
    val planned: List<String> = emptyList(),
)

/**
 * Один заход в этап; [endedAtMs] null — этап ещё идёт (или задача кончилась, не закрыв его).
 * Подряд идущие stage-события одного этапа — один заход. [note] — короткий итог шагов этапа для
 * «Инфо» («0°: 5 прогонов · 180°: из кэша», «сетка 0,25 → 0,2 мм»); null — шагов не было.
 */
data class StageSpan(
    val id: String,
    val attempt: Int,
    val startedAtMs: Long,
    val endedAtMs: Long? = null,
    val note: String? = null,
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
    /** Последнее layout-событие пакера (сырой JSON); живёт и после успеха — по нему рисуется стол. */
    val layout: String? = null,
)
