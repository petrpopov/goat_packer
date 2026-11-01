package com.goatpacker.web

import com.fasterxml.jackson.databind.JsonNode
import com.fasterxml.jackson.databind.ObjectMapper
import com.jayway.jsonpath.JsonPath
import org.springframework.test.web.servlet.MockMvc
import org.springframework.test.web.servlet.request.MockMvcRequestBuilders.get
import org.springframework.test.web.servlet.result.MockMvcResultMatchers.status
import java.io.ByteArrayOutputStream
import java.nio.file.Files
import java.nio.file.Path
import java.util.zip.CRC32
import java.util.zip.ZipEntry
import java.util.zip.ZipOutputStream

private const val POLL_INTERVAL_MS = 150L
private const val DEFAULT_TIMEOUT_MS = 30_000L

/** Установка фейковых CLI-бинарников во временный каталог (с битом исполнения). */
object FakeBins {
    /**
     * Gradle-ресурсы теряют бит исполнения, поэтому скрипт копируется из classpath во временный
     * каталог и chmod'ится явно.
     */
    fun install(root: Path, resourceName: String, targetName: String = resourceName): String {
        val bytes = requireNotNull(FakeBins::class.java.getResourceAsStream("/bin/$resourceName")) {
            "нет ресурса /bin/$resourceName"
        }.readBytes()
        val target = root.resolve(targetName)
        Files.createDirectories(target.toAbsolutePath().parent)
        Files.write(target, bytes)
        target.toFile().setReadable(true, false)
        target.toFile().setExecutable(true, false)

        return target.toAbsolutePath().toString()
    }
}

/**
 * Минимальный валидный 3MF-проект в памяти: zip с 3D/3dmodel.model и
 * Metadata/project_settings.config → printer_model. Проходит предпроверки PackService
 * (zip-структура + соответствие профиля принтеру в файле).
 */
fun miniProject3mf(printerModel: String): ByteArray {
    val bytes = ByteArrayOutputStream()
    ZipOutputStream(bytes).use { zip ->
        zip.putNextEntry(ZipEntry("3D/3dmodel.model"))
        zip.write("""<?xml version="1.0"?><model/>""".toByteArray())
        zip.closeEntry()
        zip.putNextEntry(ZipEntry("Metadata/project_settings.config"))
        zip.write("""{"printer_model":"$printerModel"}""".toByteArray())
        zip.closeEntry()
    }
    return bytes.toByteArray()
}

/** Поллинг статуса джоба до любого терминального состояния (SUCCEEDED или FAILED). */
fun MockMvc.awaitJobTerminal(id: String, timeoutMs: Long = DEFAULT_TIMEOUT_MS, mapper: ObjectMapper = ObjectMapper()): JsonNode {
    val deadline = System.currentTimeMillis() + timeoutMs
    while (System.currentTimeMillis() < deadline) {
        val body = mapper.readTree(perform(get("/api/jobs/$id")).andReturn().response.contentAsString)
        if (body.path("status").asText() in setOf("SUCCEEDED", "FAILED")) return body
        Thread.sleep(POLL_INTERVAL_MS)
    }
    throw AssertionError("Джоба $id не завершилась за $timeoutMs мс")
}

/** Поллинг статуса джоба до терминального состояния (таймаут 30 с). */
fun MockMvc.awaitJobCompletion(
    id: String,
    timeoutMs: Long = DEFAULT_TIMEOUT_MS,
    mapper: ObjectMapper = ObjectMapper(),
): JsonNode {
    val deadline = System.currentTimeMillis() + timeoutMs
    var lastBody = ""
    while (System.currentTimeMillis() < deadline) {
        lastBody = perform(get("/api/jobs/$id")).andExpect(status().isOk).andReturn().response.contentAsString
        when (JsonPath.read<String>(lastBody, "$.status")) {
            "SUCCEEDED" -> return mapper.readTree(lastBody)
            "FAILED" -> throw AssertionError("Джоба $id провалилась: $lastBody")
        }
        Thread.sleep(POLL_INTERVAL_MS)
    }
    throw AssertionError("Джоба $id не завершилась за $timeoutMs мс; последний статус: $lastBody")
}

/**
 * Minimal central-directory ZIP with data descriptors, as emitted for slicer PNGs.
 * Deliberately NOT ZipOutputStream: that writer refuses this valid combination.
 */
fun descriptorProject3mf(printerModel: String, settings: Map<String, Any> = emptyMap()): ByteArray {
    val out = ByteArrayOutputStream()

    fun word(value: Long, bytes: Int) {
        repeat(bytes) { out.write((value shr (it * 8)).toInt() and 255) }
    }

    class Entry(val name: ByteArray, val data: ByteArray, val crc: Long, val offset: Int)

    val contents = linkedMapOf(
        "Metadata/plate_1.png" to "synthetic preview",
        "3D/3dmodel.model" to "<model/>",
        "Metadata/project_settings.config" to ObjectMapper().writeValueAsString(mapOf("printer_model" to printerModel) + settings),
    )
    val entries = contents.map { (name, text) ->
        val nameBytes = name.toByteArray()
        val data = text.toByteArray()
        val crc = CRC32().apply { update(data) }.value
        val entry = Entry(nameBytes, data, crc, out.size())
        // Local file header: flag 8 = sizes and CRC follow the data in a descriptor.
        word(0x04034b50, 4); word(20, 2); word(8, 2); word(0, 2); word(0, 4)
        word(0, 4); word(0, 4); word(0, 4); word(nameBytes.size.toLong(), 2); word(0, 2)
        out.write(nameBytes)
        out.write(data)
        word(0x08074b50, 4); word(crc, 4); word(data.size.toLong(), 4); word(data.size.toLong(), 4)
        entry
    }
    val centralStart = out.size()
    entries.forEach { entry ->
        word(0x02014b50, 4); word(20, 2); word(20, 2); word(8, 2); word(0, 2); word(0, 4)
        word(entry.crc, 4); word(entry.data.size.toLong(), 4); word(entry.data.size.toLong(), 4)
        word(entry.name.size.toLong(), 2); word(0, 2); word(0, 2); word(0, 2); word(0, 2); word(0, 4)
        word(entry.offset.toLong(), 4)
        out.write(entry.name)
    }
    val centralLength = out.size() - centralStart
    word(0x06054b50, 4); word(0, 2); word(0, 2); word(entries.size.toLong(), 2); word(entries.size.toLong(), 2)
    word(centralLength.toLong(), 4); word(centralStart.toLong(), 4); word(0, 2)
    return out.toByteArray()
}
