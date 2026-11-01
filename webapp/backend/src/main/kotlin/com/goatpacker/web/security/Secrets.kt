package com.goatpacker.web.security

import java.security.MessageDigest
import java.security.SecureRandom
import java.util.Base64

/** Случайные токены и их хеши: в БД хранятся только хеши. */
object Secrets {
    private const val TOKEN_BYTES = 32
    private val random = SecureRandom()

    fun token(): String =
        Base64.getUrlEncoder().withoutPadding().encodeToString(ByteArray(TOKEN_BYTES).also(random::nextBytes))

    fun hash(value: String): String = MessageDigest.getInstance("SHA-256").digest(value.toByteArray()).toHex()
}

/** Строчные шестнадцатеричные цифры байтов (так хранятся sha256 блобов и хеши токенов). */
fun ByteArray.toHex(): String = joinToString("") { "%02x".format(it) }
