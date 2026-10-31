package com.goatpacker.web.service

import com.goatpacker.web.exception.BadRequestException

/**
 * Загрузки бывают двух видов: проект слайсера (.3mf) и голая сетка (.stl). STL уже принимается
 * загрузкой и автоопределением — фронт по нему показывает выбор принтера, — но упаковать его
 * пока нечем: нужен шаг конвертации STL → 3MF (бинарник source_project, план 2026-11-01 Task 4
 * Step 6). До него джоб с STL отклоняется при создании одной понятной 400.
 */
object StlInputs {
    const val UPLOAD_EXTENSIONS_MESSAGE = "Нужен файл .3mf или .stl"
    const val PACK_PENDING_MESSAGE = "Упаковка STL появится в ближайшем обновлении"

    fun isStl(name: String?): Boolean = name.orEmpty().trim().endsWith(".stl", ignoreCase = true)

    fun isAcceptedUpload(name: String?): Boolean =
        isStl(name) || name.orEmpty().trim().endsWith(".3mf", ignoreCase = true)

    /**
     * ВРЕМЕННЫЙ запрет: единственное место, где STL-джоб отклоняется (вызывается из локального
     * PackService и серверного DurableQueue). Удалить вместе с вызовами, когда появится шаг
     * конвертации source_project.
     */
    fun rejectUntilConversion(name: String?) {
        if (isStl(name)) throw BadRequestException(PACK_PENDING_MESSAGE)
    }
}
