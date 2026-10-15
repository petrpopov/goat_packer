package com.goatpacker.web.exception

import org.slf4j.LoggerFactory
import org.springframework.http.HttpStatus
import org.springframework.http.ResponseEntity
import org.springframework.web.bind.MethodArgumentNotValidException
import org.springframework.web.bind.annotation.ExceptionHandler
import org.springframework.web.bind.annotation.RestControllerAdvice
import org.springframework.web.multipart.MaxUploadSizeExceededException
import java.io.IOException

/** Глобальный обработчик ошибок REST API: плоские map'ы {"code", "message", "error"} на русском. */
@RestControllerAdvice
class GlobalExceptionHandler {
    companion object {
        private val log = LoggerFactory.getLogger(GlobalExceptionHandler::class.java)
    }

    @ExceptionHandler(BadRequestException::class)
    fun handleBadRequest(ex: BadRequestException): ResponseEntity<Map<String, String>> {
        val message = ex.message ?: "Некорректный запрос"
        log.warn("[BadRequest] {}", message)

        return ResponseEntity.status(HttpStatus.BAD_REQUEST)
            .body(mapOf("code" to "BAD_REQUEST", "message" to message, "error" to message))
    }

    @ExceptionHandler(ResourceNotFoundException::class)
    fun handleNotFound(ex: ResourceNotFoundException): ResponseEntity<Map<String, String>> {
        val message = ex.message ?: "Ресурс не найден"
        log.warn("[NotFound] {}", message)

        return ResponseEntity.status(HttpStatus.NOT_FOUND)
            .body(mapOf("code" to "NOT_FOUND", "message" to message, "error" to message))
    }

    @ExceptionHandler(MethodArgumentNotValidException::class)
    fun handleValidation(ex: MethodArgumentNotValidException): ResponseEntity<Map<String, Any>> {
        val errors = ex.bindingResult.fieldErrors.map { err ->
            mapOf("field" to err.field, "message" to (err.defaultMessage ?: "некорректное значение"))
        }
        val message = errors.firstOrNull()?.let { err -> "${err["field"]}: ${err["message"]}" }
            ?: "Проверьте корректность полей запроса"
        log.warn("[Validation] {}", message)

        return ResponseEntity.status(HttpStatus.BAD_REQUEST)
            .body(mapOf("code" to "VALIDATION_ERROR", "message" to message, "error" to message, "errors" to errors))
    }

    @ExceptionHandler(MaxUploadSizeExceededException::class)
    fun handleMaxUpload(ex: MaxUploadSizeExceededException): ResponseEntity<Map<String, String>> {
        log.warn("[UploadTooLarge] {}", ex.message)

        val message = "Загружаемый файл слишком большой: допустимо не более 200 МБ"
        return ResponseEntity.status(HttpStatus.PAYLOAD_TOO_LARGE)
            .body(mapOf("code" to "FILE_TOO_LARGE", "message" to message, "error" to message))
    }

    @ExceptionHandler(org.springframework.web.bind.MissingServletRequestParameterException::class,
        org.springframework.web.multipart.support.MissingServletRequestPartException::class,
        org.springframework.beans.TypeMismatchException::class,
        org.springframework.http.converter.HttpMessageNotReadableException::class)
    fun handleMalformed(ex: Exception): ResponseEntity<Map<String, String>> =
        ResponseEntity.badRequest().body(mapOf("code" to "BAD_REQUEST", "message" to "Проверьте обязательные поля и формат запроса", "error" to "Некорректный запрос"))

    @ExceptionHandler(IOException::class)
    fun handleIo(ex: IOException): ResponseEntity<Map<String, String>> {
        log.error("[IO] {}", ex.message, ex)

        val message = "Ошибка ввода-вывода: ${ex.message ?: "неизвестная ошибка"}"
        return ResponseEntity.status(HttpStatus.INTERNAL_SERVER_ERROR)
            .body(mapOf("code" to "IO_ERROR", "message" to message, "error" to message))
    }

    @ExceptionHandler(Exception::class)
    fun handleGeneric(ex: Exception): ResponseEntity<Map<String, String>> {
        log.error("[Internal] {}", ex.message, ex)

        val message = ex.message ?: "Внутренняя ошибка сервера"
        return ResponseEntity.status(HttpStatus.INTERNAL_SERVER_ERROR)
            .body(mapOf("code" to "INTERNAL_ERROR", "message" to message, "error" to message))
    }
}

class BadRequestException(message: String) : RuntimeException(message)
class ResourceNotFoundException(message: String) : RuntimeException(message)
