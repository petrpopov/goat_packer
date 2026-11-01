package com.goatpacker.web.repository

import java.nio.file.Files
import java.nio.file.Path
import java.nio.file.StandardCopyOption

/** Хранилище неизменяемых объектов (входы и результаты задач). */
interface ObjectStore {
    fun put(key: String, source: Path)

    fun get(key: String, target: Path)

    fun delete(key: String)

    /** Временная ссылка на скачивание с именем [filename]; null — хранилище не умеет подписывать ссылки. */
    fun signedGet(key: String, filename: String, seconds: Long): String?
}

/** Хранилище в каталоге: для тестов и разработки без S3. */
class FileObjectStore(private val root: Path) : ObjectStore {
    override fun put(key: String, source: Path) {
        val target = resolve(key)
        Files.createDirectories(target.parent)
        Files.copy(source, target, StandardCopyOption.REPLACE_EXISTING)
    }

    override fun get(key: String, target: Path) {
        Files.copy(resolve(key), target, StandardCopyOption.REPLACE_EXISTING)
    }

    override fun delete(key: String) {
        Files.deleteIfExists(resolve(key))
    }

    override fun signedGet(key: String, filename: String, seconds: Long): String? = null

    private fun resolve(key: String): Path {
        val path = root.resolve(key).normalize()
        require(path.startsWith(root.normalize()))
        return path
    }
}
