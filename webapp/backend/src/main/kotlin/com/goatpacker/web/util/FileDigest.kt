package com.goatpacker.web.util

import java.nio.file.Files
import java.nio.file.Path
import java.security.MessageDigest

/** Размер буфера при потоковом чтении и копировании файлов. */
const val STREAM_BUFFER_BYTES = 64 * 1024

/** Дайджест содержимого файла, прочитанного потоком (файл целиком в память не грузится). */
fun Path.digest(algorithm: String): ByteArray {
    val digest = MessageDigest.getInstance(algorithm)
    Files.newInputStream(this).use { input ->
        val buffer = ByteArray(STREAM_BUFFER_BYTES)
        while (true) {
            val read = input.read(buffer)
            if (read < 0) break
            digest.update(buffer, 0, read)
        }
    }
    return digest.digest()
}
