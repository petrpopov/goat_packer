package com.goatpacker.web.exception

import org.slf4j.LoggerFactory
import org.springframework.beans.TypeMismatchException
import org.springframework.http.HttpHeaders
import org.springframework.http.HttpStatus
import org.springframework.http.ResponseEntity
import org.springframework.http.converter.HttpMessageNotReadableException
import org.springframework.web.bind.MethodArgumentNotValidException
import org.springframework.web.bind.MissingServletRequestParameterException
import org.springframework.web.bind.annotation.ExceptionHandler
import org.springframework.web.bind.annotation.RestControllerAdvice
import org.springframework.web.multipart.MaxUploadSizeExceededException
import org.springframework.web.multipart.support.MissingServletRequestPartException
import java.io.IOException

/** Глобальный обработчик ошибок REST API: плоские map'ы {"code", "message", "error"} на русском. */
@RestControllerAdvice
class GlobalExceptionHandler {
    private val log = LoggerFactory.getLogger(javaClass)

    @ExceptionHandler(ApiException::class)
    fun handleApi(ex: ApiException): ResponseEntity<Map<String, String>> {
        log.warn("[{}] {}", ex.code, ex.message)
        val headers = HttpHeaders()
        if (ex.status == HttpStatus.TOO_MANY_REQUESTS) headers.set(HttpHeaders.RETRY_AFTER, RETRY_AFTER_SECONDS)
        return ResponseEntity.status(ex.status).headers(headers).body(errorBody(ex.code, ex.message.orEmpty()))
    }

    @ExceptionHandler(MethodArgumentNotValidException::class)
    fun handleValidation(ex: MethodArgumentNotValidException): ResponseEntity<Map<String, Any>> {
        val errors = ex.bindingResult.fieldErrors.map { err ->
            mapOf("field" to err.field, "message" to (err.defaultMessage ?: "некорректное значение"))
        }
        val message = errors.firstOrNull()?.let { err -> "${err["field"]}: ${err["message"]}" }
            ?: "Проверьте корректность полей запроса"
        log.warn("[Validation] {}", message)

        return ResponseEntity.badRequest().body(errorBody(ErrorCode.VALIDATION_ERROR, message) + ("errors" to errors))
    }

    @ExceptionHandler(MaxUploadSizeExceededException::class)
    fun handleMaxUpload(ex: MaxUploadSizeExceededException): ResponseEntity<Map<String, String>> {
        log.warn("[UploadTooLarge] {}", ex.message)

        return ResponseEntity.status(HttpStatus.PAYLOAD_TOO_LARGE)
            .body(errorBody(ErrorCode.FILE_TOO_LARGE, "Загружаемый файл слишком большой: допустимо не более 200 МБ"))
    }

    @ExceptionHandler(
        MissingServletRequestParameterException::class,
        MissingServletRequestPartException::class,
        TypeMismatchException::class,
        HttpMessageNotReadableException::class,
    )
    fun handleMalformed(): ResponseEntity<Map<String, String>> = ResponseEntity.badRequest().body(
        mapOf(
            "code" to ErrorCode.BAD_REQUEST,
            "message" to "Проверьте обязательные поля и формат запроса",
            "error" to "Некорректный запрос",
        ),
    )

    @ExceptionHandler(IOException::class)
    fun handleIo(ex: IOException): ResponseEntity<Map<String, String>> {
        log.error("[IO] {}", ex.message, ex)

        return ResponseEntity.status(HttpStatus.INTERNAL_SERVER_ERROR)
            .body(errorBody(ErrorCode.IO_ERROR, "Ошибка ввода-вывода: ${ex.message ?: "неизвестная ошибка"}"))
    }

    @ExceptionHandler(Exception::class)
    fun handleGeneric(ex: Exception): ResponseEntity<Map<String, String>> {
        log.error("[Internal] {}", ex.message, ex)

        return ResponseEntity.status(HttpStatus.INTERNAL_SERVER_ERROR)
            .body(errorBody(ErrorCode.INTERNAL_ERROR, ex.message ?: "Внутренняя ошибка сервера"))
    }

    private fun errorBody(code: String, message: String) = mapOf("code" to code, "message" to message, "error" to message)

    private companion object {
        /** Подсказка клиенту при 429: окно лимитов — минута. */
        const val RETRY_AFTER_SECONDS = "60"
    }
}
