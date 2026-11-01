package com.goatpacker.web.service.server

import com.goatpacker.web.util.SECONDS_PER_DAY
import com.goatpacker.web.util.SECONDS_PER_HOUR

/** Сроки хранения и ожидания: что и через сколько уходит из сервиса. */
object RetentionPolicy {
    /** Гостевые задачи и файлы живут сутки после завершения; у пользователей срока нет. */
    const val GUEST_CONTENT_TTL_SECONDS = SECONDS_PER_DAY

    /** Задача, не взятая исполнителем за это время, снимается с очереди. */
    const val QUEUE_WAIT_LIMIT_SECONDS = 2 * SECONDS_PER_HOUR

    /** Диагностические артефакты хранятся неделю, не сокращая жизнь основных результатов. */
    const val DEBUG_ARTIFACT_DAYS = 7

    /** Гость без сессий, файлов и задач, не появлявшийся столько дней, удаляется. */
    const val IDLE_GUEST_DAYS = 7

    const val COMPUTE_BUDGET_HISTORY_DAYS = 7
    const val REQUEST_LIMIT_HISTORY_DAYS = 2

    /** Сколько записей обрабатывается за один проход уборки. */
    const val BATCH_SIZE = 100
}
