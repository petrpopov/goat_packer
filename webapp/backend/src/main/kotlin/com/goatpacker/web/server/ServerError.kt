package com.goatpacker.web.server

import org.springframework.context.annotation.Profile
import org.springframework.http.ResponseEntity
import org.springframework.core.annotation.Order
import org.springframework.web.bind.annotation.*

class ServerError(val status: Int, val code: String, override val message: String): RuntimeException(message)
fun bad(message:String): Nothing = throw ServerError(400,"BAD_REQUEST",message)
fun missing(): Nothing = throw ServerError(404,"NOT_FOUND","Ресурс не найден")
fun limited(message:String): Nothing = throw ServerError(429,"QUOTA_EXCEEDED",message)

@RestControllerAdvice @Profile("server") @Order(-10)
class ServerErrors {
    @ExceptionHandler(ServerError::class)
    fun handle(ex: ServerError): ResponseEntity<Map<String,String>> = ResponseEntity.status(ex.status)
        .headers { if (ex.status==429) it.set("Retry-After","60") }
        .body(mapOf("code" to ex.code,"message" to ex.message,"error" to ex.message))
}
