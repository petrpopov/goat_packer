package com.goatpacker.web.service.runner

import com.fasterxml.jackson.databind.JsonNode
import com.fasterxml.jackson.databind.ObjectMapper

/**
 * Файлы кэша пакера (`--cache-dir`): имена и проверка содержимого. Раннер недоверенный, поэтому всё, что он
 * вернул, проходит [isValid], прежде чем попасть в хранилище, а имя файла — [isEntryName], прежде чем попасть в
 * команду (имя из белого списка, спецсимволов в нём нет).
 */
object ProbeCacheEntries {
    /** Каталог кэша в каталоге джоба (в контейнере — на его tmpfs). */
    const val DIR = "cache"

    private val PROBE_NAME = Regex("probe-([0-9a-f]{64})\\.json")
    private val SUPPORT_NAME = Regex("support-([0-9a-f]{64})\\.json")
    private const val MAX_RUNS = 1000

    /** Пробы класса поворота (`probe_cache.rs`) и ансамбли модели поддержек (`support_model/cache.rs`). */
    fun isEntryName(name: String): Boolean = PROBE_NAME.matches(name) || SUPPORT_NAME.matches(name)

    /** Содержимое [bytes] — ожидаемый JSON этого файла: пробы хранят свой ключ, ансамбль — два массива. */
    fun isValid(name: String, bytes: ByteArray, mapper: ObjectMapper): Boolean {
        val probeKey = PROBE_NAME.matchEntire(name)?.groupValues?.get(1)
        val supportKey = SUPPORT_NAME.matchEntire(name)?.groupValues?.get(1)
        if (probeKey == null && supportKey == null) return false
        val root = runCatching { mapper.readTree(bytes) }.getOrNull() ?: return false
        if (!root.isObject) return false
        return if (probeKey != null) isProbe(root, probeKey) else root.path("members").isArray && root.path("realizations").isArray
    }

    private fun isProbe(root: JsonNode, key: String): Boolean =
        root.path("cache_key").asText() == key &&
            root.path("runs").let { it.isIntegralNumber && it.asInt() in 1..MAX_RUNS } &&
            root.path("rotation_deg").isNumber
}
