package com.goatpacker.web.service.server

import com.fasterxml.jackson.databind.ObjectMapper
import com.goatpacker.web.entity.QueueJobEntity
import com.goatpacker.web.model.JobSnapshot
import com.goatpacker.web.model.JobStatus
import java.util.concurrent.TimeUnit

/**
 * Передаёт прогресс джоба в БД. Не чаще раза в секунду, но смена этапа пишется сразу: после неё вывода может
 * не быть долго (сборка, проверка G-code), и без записи степпер стоял бы на прошлом этапе. Новая раскладка —
 * тоже сразу и один раз (строка крупная, повторно её в БД не гоняем).
 */
class WorkerProgressReporter(
    private val execution: JobExecutionService,
    private val record: QueueJobEntity,
    private val mapper: ObjectMapper,
) {
    private var lastWriteNanos = 0L
    private var lastStage: Pair<String, Int>? = null
    private var lastLayout: String? = null

    fun report(snapshot: JobSnapshot) {
        val stageKey = snapshot.stage?.let { it.id to it.attempt }
        val newLayout = snapshot.layout?.takeIf { it != lastLayout }
        val due = System.nanoTime() - lastWriteNanos > MIN_INTERVAL_NANOS
        if (!due && snapshot.status == JobStatus.RUNNING && stageKey == lastStage && newLayout == null) return

        lastWriteNanos = System.nanoTime()
        lastStage = stageKey
        val written = execution.reportProgress(
            record.id,
            record.fence,
            snapshot.phase.jsonValue(),
            snapshot.progressPercent,
            snapshot.phaseMessage,
            snapshot.stage?.let(mapper::writeValueAsString),
            newLayout,
        )
        // lastLayout двигаем, только если запись состоялась (fence/статус могли отклонить её) — иначе раскладка потеряется.
        if (written && newLayout != null) lastLayout = newLayout
    }

    private companion object {
        val MIN_INTERVAL_NANOS = TimeUnit.SECONDS.toNanos(1)
    }
}
