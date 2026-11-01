package com.goatpacker.web.service

import com.fasterxml.jackson.databind.JsonNode
import com.fasterxml.jackson.databind.ObjectMapper
import com.goatpacker.web.exception.BadRequestException
import com.goatpacker.web.exception.ErrorMessages
import com.goatpacker.web.exception.badRequest
import com.goatpacker.web.model.Inspection
import org.springframework.stereotype.Service
import org.springframework.web.multipart.MultipartFile
import java.io.FilterInputStream
import java.io.InputStream
import java.nio.file.Files
import java.nio.file.Path
import java.nio.file.StandardCopyOption
import java.util.concurrent.TimeUnit
import java.util.zip.ZipEntry
import java.util.zip.ZipFile
import javax.xml.stream.XMLInputFactory
import javax.xml.stream.XMLResolver
import javax.xml.stream.XMLStreamConstants

/**
 * Безопасный разбор 3MF-проекта. Читается central directory, а не локальные заголовки ZIP: слайсеры
 * кладут PNG-превью как STORED + data descriptors, которые ZipInputStream отвергает до того,
 * как доберётся до настроек. Архив проверяется на «бомбы» и подмену путей, XML — на DTD и внешние сущности.
 */
@Service
class Project3mf(private val mapper: ObjectMapper) {
    fun inspect(file: MultipartFile): Inspection {
        if (file.isEmpty) badRequest("Файл .3mf пуст или не выбран")
        if (!file.originalFilename.orEmpty().endsWith(".3mf", ignoreCase = true)) badRequest("Файл должен иметь расширение .3mf")
        if (file.size > MAX_UPLOAD_BYTES) badRequest(ErrorMessages.FILE_OVER_LIMIT)
        val temporary = Files.createTempFile("goat-inspect-", ".3mf")
        try {
            file.inputStream.use { Files.copy(it, temporary, StandardCopyOption.REPLACE_EXISTING) }
            return inspect(temporary)
        } finally {
            Files.deleteIfExists(temporary)
        }
    }

    fun inspect(path: Path): Inspection = try {
        ZipFile(path.toFile()).use { zip ->
            validateEntries(zip)
            val settings = zip.getEntry(SETTINGS_ENTRY)?.let { readSettings(zip, it) }
            val model = zip.getEntry(MODEL_ENTRY)
            Inspection(
                hasModel = model != null && !model.isDirectory && model.size != 0L,
                printerModel = settings?.let(::printerModelOf),
                projectBed = settings?.let(OrcaBed::fromSettings),
            )
        }
    } catch (ex: BadRequestException) {
        throw ex
    } catch (ex: Exception) {
        badRequest("Не удалось прочитать 3MF: повреждён ZIP-архив или JSON настроек проекта")
    }

    fun requireModel(inspection: Inspection) {
        if (!inspection.hasModel) badRequest("В 3MF нет модели ($MODEL_ENTRY). Нужен проект слайсера, а не файл только с G-code")
    }

    /** Проверяет имена, размеры и XML всех записей архива. */
    private fun validateEntries(zip: ZipFile) {
        if (zip.size() > MAX_ENTRIES) badRequest("3MF содержит слишком много файлов")
        val seenNames = HashSet<String>()
        var unpacked = 0L
        val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(XML_SCAN_SECONDS)
        val xmlFactory = safeXmlFactory()
        for (entry in zip.entries()) {
            if (!isSafeName(entry.name) || !seenNames.add(entry.name.lowercase())) {
                badRequest("Недопустимые или повторяющиеся пути внутри 3MF")
            }
            if (entry.isDirectory) continue
            if (entry.size < 0 || entry.size > MAX_ENTRY_BYTES) badRequest("Слишком большой файл внутри 3MF")
            unpacked += entry.size
            if (unpacked > MAX_UNPACKED_BYTES) badRequest("Распакованный 3MF превышает 1 ГиБ")
            if (entry.name.endsWith(".model", ignoreCase = true) || entry.name.endsWith(".xml", ignoreCase = true)) {
                scanXml(zip, entry, xmlFactory, deadline)
            }
        }
    }

    private fun isSafeName(name: String): Boolean =
        name.length <= MAX_NAME_LENGTH && !name.startsWith("/") && '\\' !in name && ':' !in name &&
            name.split('/').none { it == ".." || it == "." } && name.none { it.code < SPACE_CODE }

    /** XML читается до конца без разбора содержимого: ловит DTD, лишнюю сложность и «бомбы» расширения. */
    private fun scanXml(zip: ZipFile, entry: ZipEntry, factory: XMLInputFactory, deadline: Long) {
        zip.getInputStream(entry).use { raw ->
            val reader = factory.createXMLStreamReader(LimitedXmlStream(raw, deadline))
            try {
                while (reader.hasNext()) {
                    if (reader.next() == XMLStreamConstants.DTD) badRequest("DTD/ENTITY в 3MF запрещены")
                }
            } finally {
                reader.close()
            }
        }
    }

    private fun safeXmlFactory(): XMLInputFactory = XMLInputFactory.newFactory().apply {
        setProperty(XMLInputFactory.SUPPORT_DTD, false)
        setProperty("javax.xml.stream.isSupportingExternalEntities", false)
        xmlResolver = XMLResolver { _, _, _, _ -> badRequest("Внешние XML-ресурсы запрещены") }
    }

    private fun readSettings(zip: ZipFile, entry: ZipEntry): JsonNode {
        if (entry.size > MAX_SETTINGS_BYTES) badRequest(SETTINGS_TOO_LARGE)
        val bytes = zip.getInputStream(entry).use { it.readNBytes(MAX_SETTINGS_BYTES + 1) }
        if (bytes.size > MAX_SETTINGS_BYTES) badRequest(SETTINGS_TOO_LARGE)
        val node = mapper.readTree(bytes)
        if (node == null || !node.isObject) badRequest("Настройки проекта 3MF должны быть JSON-объектом")
        for (key in EXECUTABLE_SETTINGS) {
            if (isActive(node.get(key))) badRequest("Внешние post-processing scripts в серверном режиме запрещены")
        }
        return node
    }

    /** Скрипт постобработки задан (непустая строка или массив с непустым элементом). */
    private fun isActive(setting: JsonNode?): Boolean {
        if (setting == null || setting.isNull) return false
        return if (setting.isArray) setting.any { it.asText().isNotBlank() } else setting.asText().isNotBlank()
    }

    private fun printerModelOf(settings: JsonNode): String? {
        val value = settings.get("printer_model")
        if (value != null && !value.isNull && !value.isTextual) badRequest("printer_model в 3MF должен быть строкой")
        return value?.asText()?.trim()?.takeIf { it.isNotEmpty() }
    }

    /** Читает XML с общим дедлайном и лимитом распакованных байт. */
    private class LimitedXmlStream(raw: InputStream, private val deadline: Long) : FilterInputStream(raw) {
        private var bytes = 0L

        override fun read(b: ByteArray, off: Int, len: Int): Int {
            if (System.nanoTime() > deadline) badRequest("Слишком сложный XML-проект")
            val read = super.read(b, off, len)
            if (read > 0) bytes += read
            if (bytes > MAX_ENTRY_BYTES) badRequest("XML превышает лимит распаковки")
            return read
        }
    }

    companion object {
        /** Предел размера загружаемого файла (совпадает с `spring.servlet.multipart` локального режима). */
        const val MAX_UPLOAD_BYTES = 200L * 1024 * 1024

        private const val SETTINGS_ENTRY = "Metadata/project_settings.config"
        private const val MODEL_ENTRY = "3D/3dmodel.model"
        private const val MAX_ENTRIES = 10_000
        private const val MAX_NAME_LENGTH = 500
        private const val SPACE_CODE = 32
        private const val MAX_ENTRY_BYTES = 256L * 1024 * 1024
        private const val MAX_UNPACKED_BYTES = 1024L * 1024 * 1024
        private const val MAX_SETTINGS_BYTES = 8 * 1024 * 1024
        private const val SETTINGS_TOO_LARGE = "Настройки принтера в 3MF превышают 8 МБ"
        private const val XML_SCAN_SECONDS = 15L
        private val EXECUTABLE_SETTINGS = listOf("post_process", "post_process_script", "post_process_scripts")
    }
}
