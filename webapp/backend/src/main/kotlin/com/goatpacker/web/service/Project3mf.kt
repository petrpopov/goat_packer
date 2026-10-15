package com.goatpacker.web.service

import com.fasterxml.jackson.databind.ObjectMapper
import com.goatpacker.web.exception.BadRequestException
import org.springframework.stereotype.Service
import org.springframework.web.multipart.MultipartFile
import java.nio.file.Files
import java.nio.file.Path
import java.util.zip.ZipFile

/** Read the central directory, not local ZIP headers: slicers use STORED + data descriptors
 * for PNG previews, which java.util.zip.ZipInputStream rejects before reaching the settings. */
@Service
class Project3mf(private val mapper: ObjectMapper) {
    data class Inspection(val hasModel: Boolean, val printerModel: String?)

    fun inspect(file: MultipartFile): Inspection {
        if (file.isEmpty) throw BadRequestException("Файл .3mf пуст или не выбран")
        if (!file.originalFilename.orEmpty().endsWith(".3mf", ignoreCase = true)) {
            throw BadRequestException("Файл должен иметь расширение .3mf")
        }
        if (file.size > 200L * 1024 * 1024) throw BadRequestException("Файл больше 200 МБ")
        val temporary = Files.createTempFile("goat-inspect-", ".3mf")
        try {
            file.inputStream.use { Files.copy(it, temporary, java.nio.file.StandardCopyOption.REPLACE_EXISTING) }
            return inspect(temporary)
        } finally {
            Files.deleteIfExists(temporary)
        }
    }

    fun inspect(path: Path): Inspection = try {
        ZipFile(path.toFile()).use { zip ->
            if (zip.size() > 10_000) throw BadRequestException("3MF содержит слишком много файлов")
            val names = HashSet<String>()
            var unpacked = 0L
            val deadline = System.nanoTime() + java.util.concurrent.TimeUnit.SECONDS.toNanos(15)
            val xmlFactory = javax.xml.stream.XMLInputFactory.newFactory().apply {
                setProperty(javax.xml.stream.XMLInputFactory.SUPPORT_DTD, false)
                setProperty("javax.xml.stream.isSupportingExternalEntities", false)
                xmlResolver = javax.xml.stream.XMLResolver { _, _, _, _ -> throw BadRequestException("Внешние XML-ресурсы запрещены") }
            }
            zip.entries().asSequence().forEach { entry ->
                val name = entry.name
                if (name.length > 500 || name.startsWith("/") || '\\' in name || ':' in name || name.split('/').any { it == ".." || it == "." }
                    || name.any { it.code < 32 } || !names.add(name.lowercase())) throw BadRequestException("Недопустимые или повторяющиеся пути внутри 3MF")
                if (!entry.isDirectory) {
                    if (entry.size < 0 || entry.size > 256L * 1024 * 1024) throw BadRequestException("Слишком большой файл внутри 3MF")
                    unpacked += entry.size
                    if (unpacked > 1024L * 1024 * 1024) throw BadRequestException("Распакованный 3MF превышает 1 ГиБ")
                }
                if (!entry.isDirectory && (name.endsWith(".model", true) || name.endsWith(".xml", true))) {
                    zip.getInputStream(entry).use { raw ->
                        val input = object : java.io.FilterInputStream(raw) {
                            var bytes = 0L
                            override fun read(b: ByteArray, off: Int, len: Int): Int {
                                if (System.nanoTime() > deadline) throw BadRequestException("Слишком сложный XML-проект")
                                val n = super.read(b, off, len); if (n > 0) bytes += n
                                if (bytes > 256L * 1024 * 1024) throw BadRequestException("XML превышает лимит распаковки")
                                return n
                            }
                        }
                        val reader = xmlFactory.createXMLStreamReader(input)
                        try { while (reader.hasNext()) {
                            if (reader.next() == javax.xml.stream.XMLStreamConstants.DTD) throw BadRequestException("DTD/ENTITY в 3MF запрещены")
                        } } finally { reader.close() }
                    }
                }
            }
            val settings = zip.getEntry("Metadata/project_settings.config")
            val model = zip.getEntry("3D/3dmodel.model")
            val printer = settings?.let {
                val limit = 8 * 1024 * 1024
                if (it.size > limit) throw BadRequestException("Настройки принтера в 3MF превышают 8 МБ")
                val bytes = zip.getInputStream(it).use { stream -> stream.readNBytes(limit + 1) }
                if (bytes.size > limit) throw BadRequestException("Настройки принтера в 3MF превышают 8 МБ")
                val node = mapper.readTree(bytes)
                if (node == null || !node.isObject) throw BadRequestException("Настройки проекта 3MF должны быть JSON-объектом")
                val executableSettings = listOf("post_process", "post_process_script", "post_process_scripts")
                for (key in executableSettings) {
                    val setting = node.get(key)
                    val active = setting != null && !setting.isNull && (if (setting.isArray) setting.any { item -> item.asText().isNotBlank() } else setting.asText().isNotBlank())
                    if (active) throw BadRequestException("Внешние post-processing scripts в серверном режиме запрещены")
                }
                val value = node.get("printer_model")
                if (value != null && !value.isNull && !value.isTextual) {
                    throw BadRequestException("printer_model в 3MF должен быть строкой")
                }
                value?.asText()?.trim()?.takeIf { name -> name.isNotEmpty() }
            }
            Inspection(model != null && !model.isDirectory && model.size != 0L, printer)
        }
    } catch (ex: BadRequestException) {
        throw ex
    } catch (ex: Exception) {
        throw BadRequestException("Не удалось прочитать 3MF: повреждён ZIP-архив или JSON настроек проекта")
    }

    fun requireModel(inspection: Inspection) {
        if (!inspection.hasModel) throw BadRequestException("В 3MF нет модели (3D/3dmodel.model). Нужен проект слайсера, а не файл только с G-code")
    }
}
