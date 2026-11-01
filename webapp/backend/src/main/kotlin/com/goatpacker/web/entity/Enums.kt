package com.goatpacker.web.entity

/** Гость (сессия без регистрации) или зарегистрированный пользователь. */
enum class PrincipalKind { GUEST, USER }

enum class Role { USER, ADMIN }

/** Загрузка файла: резерв места → приём байт → готово к использованию в задаче. */
enum class UploadState {
    UPLOADING,
    RECEIVING,
    READY;

    companion object {
        /** Загрузки, занимающие слот приёма и зарезервированное место. */
        val IN_PROGRESS = setOf(UPLOADING, RECEIVING)
    }
}

/** Состояние строки очереди (колонка `queue_jobs.status`). */
enum class QueueStatus {
    QUEUED,
    RUNNING,
    FINALIZING,
    LOST,
    RETRY_WAIT,
    SUCCEEDED,
    FAILED,
    CANCELLED,
    EXPIRED;

    companion object {
        /** Задача ещё не завершена: ждёт, идёт или потеряна исполнителем. */
        val ACTIVE = setOf(QUEUED, RUNNING, FINALIZING, LOST, RETRY_WAIT)

        /** Ждёт запуска (в очереди или после сбоя исполнителя). */
        val WAITING = setOf(QUEUED, RETRY_WAIT)

        /** Занимает слот исполнителя; LOST держит слот, пока остановка не подтверждена. */
        val OCCUPYING_SLOT = setOf(RUNNING, FINALIZING, LOST)

        /** Исполнитель реально работает над задачей. */
        val EXECUTING = setOf(RUNNING, FINALIZING)
    }
}
