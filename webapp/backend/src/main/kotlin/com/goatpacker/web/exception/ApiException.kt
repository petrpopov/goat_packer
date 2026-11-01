package com.goatpacker.web.exception

import org.springframework.http.HttpStatus

/** Ошибка API: статус, машинный код и русское сообщение уходят клиенту как есть. */
open class ApiException(val status: HttpStatus, val code: String, message: String) : RuntimeException(message)

class BadRequestException(message: String) : ApiException(HttpStatus.BAD_REQUEST, ErrorCode.BAD_REQUEST, message)

class ResourceNotFoundException(message: String = ErrorMessages.NOT_FOUND) :
    ApiException(HttpStatus.NOT_FOUND, ErrorCode.NOT_FOUND, message)

/** Коды ошибок — часть контракта с фронтендом. */
object ErrorCode {
    const val BAD_REQUEST = "BAD_REQUEST"
    const val NOT_FOUND = "NOT_FOUND"
    const val UNAUTHORIZED = "UNAUTHORIZED"
    const val FORBIDDEN = "FORBIDDEN"
    const val BLOCKED = "BLOCKED"
    const val CONFLICT = "CONFLICT"
    const val EXPIRED = "EXPIRED"
    const val MAINTENANCE = "MAINTENANCE"
    const val FILE_TOO_LARGE = "FILE_TOO_LARGE"
    const val BODY_TOO_LARGE = "BODY_TOO_LARGE"
    const val QUOTA_EXCEEDED = "QUOTA_EXCEEDED"
    const val VALIDATION_ERROR = "VALIDATION_ERROR"
    const val IO_ERROR = "IO_ERROR"
    const val INTERNAL_ERROR = "INTERNAL_ERROR"
}

/** Тексты ошибок, которые повторяются в нескольких местах. */
object ErrorMessages {
    const val NOT_FOUND = "Ресурс не найден"
    const val ACCESS_BLOCKED = "Доступ заблокирован"
    const val QUEUE_FULL = "Очередь задач переполнена — попробуйте повторить позже"
    const val FILE_OVER_LIMIT = "Файл больше 200 МБ"
    const val PRINTER_MODEL_MISSING = "В проекте нет printer_model. Сохраните проект с выбранным принтером в слайсере"
    const val REASON_REQUIRED = "Укажите причину (3–500 символов)"
    const val INVALID_PAGE = "Некорректная страница"
    const val TOO_MANY_REQUESTS = "Слишком много запросов — повторите позже"
    const val IDEMPOTENCY_KEY_CONFLICT = "Этот ключ запроса использован с другими параметрами"
}

fun badRequest(message: String): Nothing = throw BadRequestException(message)

fun notFound(): Nothing = throw ResourceNotFoundException()

fun quotaExceeded(message: String): Nothing = throw ApiException(HttpStatus.TOO_MANY_REQUESTS, ErrorCode.QUOTA_EXCEEDED, message)

fun accessBlocked(): Nothing = throw ApiException(HttpStatus.FORBIDDEN, ErrorCode.BLOCKED, ErrorMessages.ACCESS_BLOCKED)

fun underMaintenance(message: String): Nothing = throw ApiException(HttpStatus.SERVICE_UNAVAILABLE, ErrorCode.MAINTENANCE, message)

fun expired(message: String): Nothing = throw ApiException(HttpStatus.GONE, ErrorCode.EXPIRED, message)
