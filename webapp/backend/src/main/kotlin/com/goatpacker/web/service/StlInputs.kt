package com.goatpacker.web.service

import com.fasterxml.jackson.databind.JsonNode
import com.goatpacker.web.exception.BadRequestException

/**
 * Загрузки бывают двух видов: проект слайсера (.3mf) и голая сетка (.stl). STL перед упаковкой
 * превращается в проект бинарником source_project (STL → input.3mf, пресеты слайсера образа),
 * дальше джоб идёт как обычный 3MF.
 */
object StlInputs {
    const val UPLOAD_EXTENSIONS_MESSAGE = "Нужен файл .3mf или .stl"
    /** Сообщение этапа конвертации (до пробной нарезки пакера). */
    const val CONVERSION_MESSAGE = "Готовлю проект из STL…"
    const val LOCAL_PRESETS_MISSING = "STL в локальном режиме требует GOAT_SLICER_PRESETS"
    const val INVALID_STL = "Файл .stl повреждён или не является STL"

    /** Вход джоба лежит в каталоге под этими именами; STL конвертируется в [INPUT_3MF]. */
    const val INPUT_3MF = "input.3mf"
    const val INPUT_STL = "input.stl"

    /** Каталог черновиков source_project внутри каталога джоба (пресеты, export.log). */
    const val WORK_DIR = "stl-work"

    /** Пресеты слайсера в runner-образе (plain JSON) и CLI для экспорта: в runner-bambu это Bambu Studio. */
    const val RUNNER_PRESETS = "/opt/slicer/resources/profiles"

    fun isStl(name: String?): Boolean = name.orEmpty().trim().endsWith(".stl", ignoreCase = true)

    fun isAcceptedUpload(name: String?): Boolean =
        isStl(name) || name.orEmpty().trim().endsWith(".3mf", ignoreCase = true)

    fun inputName(originalName: String?): String = if (isStl(originalName)) INPUT_STL else INPUT_3MF

    /**
     * Имя исходника задачи, собранной из упаковки: упаковка STL отдаёт проект 3MF, и расписание по
     * ней — обычный 3MF (`cat.stl` → `cat.3mf`). Иначе воркер положил бы packed.3mf как input.stl.
     */
    fun projectName(originalName: String): String =
        if (isStl(originalName)) originalName.trim().dropLast(4) + ".3mf" else originalName

    /**
     * Дешёвая проверка, что файл похож на STL: бинарный — размер ровно 84 + 50·n (n из заголовка),
     * ASCII — начинается с `solid` и в начале есть `facet`. Бинарный проверяется первым: заголовок
     * бинарного STL тоже может начинаться со `solid`.
     */
    fun looksLikeStl(size: Long, input: java.io.InputStream): Boolean {
        val head = input.readNBytes(64 * 1024)
        if (head.size >= 84) {
            val n = java.nio.ByteBuffer.wrap(head, 80, 4).order(java.nio.ByteOrder.LITTLE_ENDIAN).int.toLong() and 0xffffffffL
            if (size == 84 + 50 * n) return true
        }
        val text = String(head, Charsets.ISO_8859_1)
        return text.trimStart().startsWith("solid") && "facet" in text
    }

    fun requireValidStl(path: java.nio.file.Path) {
        val valid = java.nio.file.Files.newInputStream(path).use { looksLikeStl(java.nio.file.Files.size(path), it) }
        if (!valid) throw BadRequestException(INVALID_STL)
    }

    fun requireValidStl(file: org.springframework.web.multipart.MultipartFile) {
        if (!file.inputStream.use { looksLikeStl(file.size, it) }) throw BadRequestException(INVALID_STL)
    }

    /**
     * Аргументы принтера для source_project из json профиля: свой стол (id [CustomProfile.ID]) —
     * `--custom-bed W,D,H`; каталожный — `--vendor` (папка пресетов) и `--machine` (orca_machine).
     * Профиль без orca_machine из STL не собрать — понятная 400 при постановке.
     */
    fun printerArgs(profile: JsonNode): List<String> {
        if (profile.path("id").asText() == CustomProfile.ID) {
            val area = profile.path("printable_area").map { it.asDouble() }
            val height = profile.path("printable_height").asDouble(0.0)
            require(area.size == 4 && height > 0) { "custom profile without printable_area/printable_height" }
            return listOf("--custom-bed", listOf(area[2] - area[0], area[3] - area[1], height).joinToString(",") { number(it) })
        }
        val vendor = profile.path("vendor").asText("").trim()
        val machine = profile.path("orca_machine").asText("").trim()
        if (vendor.isEmpty() || machine.isEmpty()) {
            throw BadRequestException("Для принтера «${profile.path("printer_name").asText("?")}» проект из STL собрать нельзя — загрузите 3MF из слайсера")
        }
        return listOf("--vendor", vendor, "--machine", machine)
    }

    /**
     * Команда source_project: [bin] — путь к бинарнику (в docker-режиме важно только имя).
     * `--edge-margin` — edge_margin_mm профиля: модель, не влезающая в стол за вычетом кромки,
     * отсекается до экспорта (пакер всё равно её не разместил бы).
     */
    fun command(bin: String, presets: String, slicer: String, profile: JsonNode): List<String> {
        val margin = profile.path("edge_margin_mm").asDouble(0.0).takeIf { it.isFinite() && it > 0 }
        return listOf(bin, "--stl", INPUT_STL, "--presets", presets, "--orca", slicer) + printerArgs(profile) +
            (margin?.let { listOf("--edge-margin", number(it)) } ?: emptyList()) +
            listOf("--output", INPUT_3MF, "--work-dir", WORK_DIR)
    }

    /**
     * Локальный режим: пресеты и слайсер экспорта из настроек. Принтер Bambu с [AppProperties.bambuPresets]
     * экспортирует Bambu Studio, иначе — OrcaSlicer из [AppProperties.slicerPresets] (Orca знает и BBL-машины,
     * кроме A2L). Без пресетов STL локально не собрать — ошибка при постановке, а не на минуте работы.
     */
    fun localTools(app: com.goatpacker.web.config.AppProperties, profile: JsonNode): Pair<String, String> {
        if (profile.path("slicer_kind").asText() == "bambu" && app.bambuPresets.isNotBlank()) return app.bambuPresets to app.bambuSlicer
        if (app.slicerPresets.isBlank()) throw BadRequestException(LOCAL_PRESETS_MISSING)
        return app.slicerPresets to app.orcaSlicer
    }

    private fun number(value: Double): String = if (value == Math.rint(value)) value.toLong().toString() else value.toString()
}
