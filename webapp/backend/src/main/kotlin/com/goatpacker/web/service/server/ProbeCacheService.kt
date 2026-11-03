package com.goatpacker.web.service.server

import com.fasterxml.jackson.databind.ObjectMapper
import com.goatpacker.web.config.PlatformProperties
import com.goatpacker.web.entity.newId
import com.goatpacker.web.repository.ObjectGcRepository
import com.goatpacker.web.repository.ObjectStore
import com.goatpacker.web.repository.ProbeCacheEntry
import com.goatpacker.web.repository.ProbeCacheRepository
import com.goatpacker.web.security.toHex
import com.goatpacker.web.service.runner.ProbeCacheEntries
import com.goatpacker.web.util.digest
import org.slf4j.LoggerFactory
import org.springframework.context.annotation.Profile
import org.springframework.stereotype.Service
import java.nio.file.Files
import java.nio.file.LinkOption
import java.nio.file.Path

/**
 * Постоянный кэш проб пакера. Раннер недоверенный и общего каталога не имеет, поэтому кэш ведёт воркер:
 * перед запуском кладёт в каталог джоба записи этого владельца для этой модели ([restore]), после —
 * проверяет и забирает новые ([harvest]). Ключ в хранилище `probe-cache/<владелец>/<модель>/<id>/<имя>`:
 * кэш одного владельца (гость — тоже владелец) недоступен чужой джобе, подмена записи вредит только себе.
 * «Модель» — SHA-256 входа (3MF/STL): пакер сам сверяет точный ключ записи (сетка, настройки, слайсер,
 * поворот), так что лишние записи группы безвредны, а пакеру не нужен режим «напечатать ключи».
 */
@Service
@Profile("server")
class ProbeCacheService(
    private val entries: ProbeCacheRepository,
    private val store: ObjectStore,
    private val gc: ObjectGcRepository,
    private val props: PlatformProperties,
    private val mapper: ObjectMapper,
) {
    private val log = LoggerFactory.getLogger(javaClass)

    val enabled: Boolean get() = props.probeCacheEnabled

    /** Группа записей: хеш входа джоба. */
    fun groupOf(input: Path): String = input.digest("SHA-256").toHex()

    /** Кладёт записи владельца для группы в [dir]; возвращает их число. Сбой одной записи — просто промах. */
    fun restore(owner: String, group: String, dir: Path): Int {
        Files.createDirectories(dir)
        val restored = entries.list(owner, group, props.probeCacheGroupMaxEntries).filter {
            runCatching { store.get(it.objectKey, dir.resolve(it.name)) }
                .onFailure { error -> log.warn("event=probe_cache_restore_failed owner={} name={} reason={}", owner, it.name, error.message) }
                .isSuccess
        }
        entries.touch(owner, group, restored.map(ProbeCacheEntry::name))
        return restored.size
    }

    /** Забирает из [dir] новые записи, прошедшие проверку; затем вытесняет лишнее. Возвращает число сохранённых. */
    fun harvest(owner: String, group: String, dir: Path): Int {
        if (!Files.isDirectory(dir, LinkOption.NOFOLLOW_LINKS)) return 0
        val known = entries.names(owner, group)
        val candidates = Files.list(dir).use { stream ->
            stream.sorted().limit(MAX_SCANNED).toList()
        }
        val stored = candidates.count { it.fileName.toString() !in known && store(owner, group, it) }
        if (stored > 0) {
            entries.evictGroup(owner, group, props.probeCacheGroupMaxEntries)
            entries.evictOwner(owner, props.probeCacheOwnerMaxBytes)
        }
        return stored
    }

    /** Записи, не использовавшиеся дольше срока, уходят (вызывается уборкой хранилища). */
    fun expire() {
        entries.evictUnusedFor(props.probeCacheTtlDays)
    }

    private fun store(owner: String, group: String, file: Path): Boolean {
        val name = file.fileName.toString()
        if (!ProbeCacheEntries.isEntryName(name) || !Files.isRegularFile(file, LinkOption.NOFOLLOW_LINKS)) return false
        val size = Files.size(file)
        if (size !in 1..props.probeCacheEntryMaxBytes) return false
        if (!ProbeCacheEntries.isValid(name, Files.readAllBytes(file), mapper)) {
            log.warn("event=probe_cache_rejected owner={} name={}", owner, name)
            return false
        }
        val key = "probe-cache/$owner/$group/${newId()}/$name"
        gc.stage(key, size)
        store.put(key, file)
        val inserted = entries.insert(owner, group, ProbeCacheEntry(name, key, size))
        // Объект либо принят строкой (GC его не тронет), либо лишний и уходит в очередь удаления.
        if (inserted) gc.remove(key) else gc.scheduleDeletion(key, size)
        return inserted
    }

    private companion object {
        /** Сколько файлов каталога просматривается: чужой мусор в нём не должен тормозить воркер. */
        const val MAX_SCANNED = 64L
    }
}
