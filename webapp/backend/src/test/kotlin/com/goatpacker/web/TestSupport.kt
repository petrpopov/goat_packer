package com.goatpacker.web

import com.fasterxml.jackson.databind.JsonNode
import com.fasterxml.jackson.databind.ObjectMapper
import com.jayway.jsonpath.JsonPath
import java.nio.file.Files
import java.nio.file.Path
import org.springframework.test.web.servlet.MockMvc
import org.springframework.test.web.servlet.request.MockMvcRequestBuilders.get

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
    val bytes = java.io.ByteArrayOutputStream()
    java.util.zip.ZipOutputStream(bytes).use { zip ->
        zip.putNextEntry(java.util.zip.ZipEntry("3D/3dmodel.model"))
        zip.write("""<?xml version="1.0"?><model/>""".toByteArray())
        zip.closeEntry()
        zip.putNextEntry(java.util.zip.ZipEntry("Metadata/project_settings.config"))
        zip.write("""{"printer_model":"$printerModel"}""".toByteArray())
        zip.closeEntry()
    }
    return bytes.toByteArray()
}

/** Поллинг статуса джоба до любого терминального состояния (SUCCEEDED или FAILED). */
fun MockMvc.awaitJobTerminal(id: String, timeoutMs: Long = 30_000, mapper: ObjectMapper = ObjectMapper()): JsonNode {
    val deadline = System.currentTimeMillis() + timeoutMs
    while (System.currentTimeMillis() < deadline) {
        val body = mapper.readTree(perform(get("/api/jobs/$id")).andReturn().response.contentAsString)
        if (body.path("status").asText() in setOf("SUCCEEDED", "FAILED")) return body
        Thread.sleep(150)
    }
    throw AssertionError("Джоба $id не завершилась за $timeoutMs мс")
}

/** Поллинг статуса джоба до терминального состояния (таймаут 30 с). */
fun MockMvc.awaitJobCompletion(
    id: String,
    timeoutMs: Long = 30_000,
    mapper: ObjectMapper = ObjectMapper(),
): JsonNode {
    val deadline = System.currentTimeMillis() + timeoutMs
    var lastBody = ""
    while (System.currentTimeMillis() < deadline) {
        lastBody = perform(get("/api/jobs/$id"))
            .andExpect(org.springframework.test.web.servlet.result.MockMvcResultMatchers.status().isOk)
            .andReturn()
            .response
            .contentAsString
        when (val status = JsonPath.read<String>(lastBody, "$.status")) {
            "SUCCEEDED" -> return mapper.readTree(lastBody)
            "FAILED" -> throw AssertionError("Джоба $id провалилась: $lastBody")
        }
        Thread.sleep(150)
    }
    throw AssertionError("Джоба $id не завершилась за $timeoutMs мс; последний статус: $lastBody")
}

/** Minimal central-directory ZIP with STORED + data descriptors, as emitted for slicer PNGs.
 * Deliberately NOT ZipOutputStream: that writer refuses this valid combination. */
fun descriptorProject3mf(printerModel: String, settings: Map<String, Any> = emptyMap()): ByteArray {
    val out = java.io.ByteArrayOutputStream()
    fun word(value: Long, count: Int) { repeat(count) { out.write((value shr (it * 8)).toInt() and 255) } }
    data class Entry(val name: ByteArray, val data: ByteArray, val crc: Long, val offset: Int)
    val entries = linkedMapOf(
        "Metadata/plate_1.png" to "synthetic preview",
        "3D/3dmodel.model" to "<model/>",
        "Metadata/project_settings.config" to ObjectMapper().writeValueAsString(mapOf("printer_model" to printerModel) + settings),
    ).map { (name, data) ->
        val n = name.toByteArray(); val bytes = data.toByteArray()
        val crc = java.util.zip.CRC32().apply { update(bytes) }.value
        val entry = Entry(n, bytes, crc, out.size())
        word(0x04034b50,4); word(20,2); word(8,2); word(0,2); word(0,4)
        word(0,4); word(0,4); word(0,4); word(n.size.toLong(),2); word(0,2)
        out.write(n); out.write(bytes)
        word(0x08074b50,4); word(crc,4); word(bytes.size.toLong(),4); word(bytes.size.toLong(),4)
        entry
    }
    val central = out.size()
    entries.forEach { e ->
        word(0x02014b50,4); word(20,2); word(20,2); word(8,2); word(0,2); word(0,4)
        word(e.crc,4); word(e.data.size.toLong(),4); word(e.data.size.toLong(),4)
        word(e.name.size.toLong(),2); word(0,2); word(0,2); word(0,2); word(0,2); word(0,4)
        word(e.offset.toLong(),4); out.write(e.name)
    }
    val length = out.size() - central
    word(0x06054b50,4); word(0,2); word(0,2); word(entries.size.toLong(),2); word(entries.size.toLong(),2)
    word(length.toLong(),4); word(central.toLong(),4); word(0,2)
    return out.toByteArray()
}
