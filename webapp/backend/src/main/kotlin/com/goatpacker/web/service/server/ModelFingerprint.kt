package com.goatpacker.web.service.server

import com.goatpacker.web.security.toHex
import com.goatpacker.web.util.digest
import java.io.FilterInputStream
import java.io.InputStream
import java.nio.ByteBuffer
import java.nio.file.Path
import java.security.MessageDigest
import java.util.zip.ZipFile
import javax.xml.stream.XMLInputFactory
import javax.xml.stream.XMLStreamConstants
import javax.xml.stream.XMLStreamReader

/**
 * Отпечаток МОДЕЛИ входа джоба: по нему группируется кэш проб. Хеш всего файла не годится: пересохранение проекта
 * в GUI слайсера меняет даты, миниатюры, `model_settings.config`, UUID и имена объектов, а модель та же.
 *
 * Проверено на файлах koza-mini/print (3MF BambuStudio/Orca): девять сохранений одного козла (P1S/A2L, разные
 * слои и профили) дают байт-в-байт один `3D/Objects/object_1.model` (CRC eac3ebba), но тот же козёл из Snapmaker
 * Orca лежит в `koza_45mm_150k_v2.obj_4.model` с другим `p:UUID` объекта, а из Bambu A2L (`object_6.model`) ещё и
 * с другим порядком атрибутов `paint_color`/`paint_supports` у треугольников. Поэтому хеш берётся не по байтам
 * части, а по разобранным числам: вершины (double), треугольники (индексы и прочие атрибуты, отсортированные по
 * имени, без `UUID`) — на всех трёх вариантах получается одно значение. Пустые части (`<resources/>`, по одной на
 * каждый «лишний» объект проекта) и `<build>` в отпечаток не входят; объекты с сеткой считаются независимо
 * (имя части и id не важны), итог — SHA-256 отсортированного множества их хешей.
 *
 * STL — сама сетка, хеш байтов. Нет ни одной сетки (не 3MF, битый архив, сбой разбора) — хеш всего файла.
 */
object ModelFingerprint {
    private const val SCHEME = "goat-model-v1"
    private const val MAX_BYTES = 2L shl 30

    fun of(input: Path, isStl: Boolean): String {
        if (isStl) return input.digest("SHA-256").toHex()
        val meshes = runCatching { meshHashes(input) }.getOrNull().orEmpty()
        if (meshes.isEmpty()) return input.digest("SHA-256").toHex()
        val total = MessageDigest.getInstance("SHA-256")
        total.update(SCHEME.toByteArray())
        meshes.sorted().forEach { total.update(it.toByteArray()) }
        return total.digest().toHex()
    }

    private fun meshHashes(input: Path): Set<String> = ZipFile(input.toFile()).use { zip ->
        val budget = longArrayOf(MAX_BYTES)
        zip.entries().asSequence()
            .filter { !it.isDirectory && it.name.trimStart('/').startsWith("3D/") && it.name.endsWith(".model") }
            .flatMap { entry -> zip.getInputStream(entry).use { meshesOf(Capped(it, budget)) } }
            .toSet()
    }

    private fun meshesOf(stream: InputStream): List<String> {
        val factory = XMLInputFactory.newInstance().apply {
            setProperty(XMLInputFactory.SUPPORT_DTD, false)
            setProperty(XMLInputFactory.IS_SUPPORTING_EXTERNAL_ENTITIES, false)
        }
        val reader = factory.createXMLStreamReader(stream)
        val found = mutableListOf<String>()
        var digest: MessageDigest? = null
        var hasMesh = false
        val number = ByteBuffer.allocate(Long.SIZE_BYTES * 3)
        try {
            while (reader.hasNext()) {
                when (reader.next()) {
                    XMLStreamConstants.START_ELEMENT -> when (reader.localName) {
                        "object" -> { digest = MessageDigest.getInstance("SHA-256"); hasMesh = false }
                        "mesh" -> hasMesh = true
                        "vertex" -> digest?.let { number.addVertex(reader, it) }
                        "triangle" -> digest?.let { addTriangle(reader, it) }
                    }
                    XMLStreamConstants.END_ELEMENT -> if (reader.localName == "object") {
                        if (hasMesh) digest?.let { found += it.digest().toHex() }
                        digest = null
                    }
                }
            }
        } finally {
            reader.close()
        }
        return found
    }

    private fun ByteBuffer.addVertex(reader: XMLStreamReader, digest: MessageDigest) {
        clear()
        for (axis in AXES) putLong(reader.getAttributeValue(null, axis).toDouble().toBits())
        digest.update(array(), 0, position())
        digest.update(VERTEX_MARK)
    }

    private fun addTriangle(reader: XMLStreamReader, digest: MessageDigest) {
        val attributes = (0 until reader.attributeCount)
            .filter { reader.getAttributeLocalName(it) != "UUID" }
            .map { reader.getAttributeLocalName(it) to reader.getAttributeValue(it) }
            .sortedBy { it.first }
        for ((name, value) in attributes) digest.update("$name=$value;".toByteArray())
        digest.update(TRIANGLE_MARK)
    }

    private val AXES = listOf("x", "y", "z")
    private val VERTEX_MARK = byteArrayOf('v'.code.toByte())
    private val TRIANGLE_MARK = byteArrayOf('t'.code.toByte())

    /** Предел распакованного объёма всех частей: бомба в архиве не должна подвесить воркер. */
    private class Capped(input: InputStream, private val budget: LongArray) : FilterInputStream(input) {
        override fun read(): Int = super.read().also { if (it >= 0) spend(1) }
        override fun read(b: ByteArray, off: Int, len: Int): Int = super.read(b, off, len).also { if (it > 0) spend(it.toLong()) }
        private fun spend(bytes: Long) {
            budget[0] -= bytes
            check(budget[0] >= 0) { "3MF model parts are too large" }
        }
    }
}
