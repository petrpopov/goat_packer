package com.goatpacker.web.util

private const val MAX_FILE_NAME_LENGTH = 180
private const val DEFAULT_FILE_NAME = "model.3mf"
private val UNSAFE_NAME_CHARS = Regex("[^\\p{L}\\p{N}._ -]")

/** Имя файла без пути и служебных символов — безопасно для заголовков, ключей хранилища и ссылок. */
fun safeName(raw: String): String = raw.substringAfterLast('/').substringAfterLast('\\')
    .replace(UNSAFE_NAME_CHARS, "_")
    .trim()
    .take(MAX_FILE_NAME_LENGTH)
    .ifBlank { DEFAULT_FILE_NAME }
