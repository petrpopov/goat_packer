package com.goatpacker.web.dto

import com.goatpacker.web.exception.BadRequestException
import kotlin.math.min

/**
 * «Моего принтера нет в списке»: стол, введённый пользователем, мм. [margin] — отступ от края
 * (по умолчанию [DEFAULT_MARGIN_MM]: где у незнакомого принтера линия прочистки, мы не знаем).
 */
data class CustomBed(
    val width: Double = 0.0,
    val depth: Double = 0.0,
    val height: Double = 0.0,
    val margin: Double = DEFAULT_MARGIN_MM,
) {
    fun validate() {
        for ((label, value) in listOf("Ширина" to width, "Глубина" to depth, "Высота" to height)) {
            if (!value.isFinite() || value !in SIZE_RANGE_MM) throw BadRequestException("$label стола должна быть 50–1000 мм")
        }
        if (!margin.isFinite() || margin !in MARGIN_RANGE_MM) throw BadRequestException("Отступ от края должен быть 0–30 мм")
        // Иначе полезной области не остаётся: профиль с такой кромкой отверг бы и пакер.
        if (margin * 2 >= min(width, depth)) throw BadRequestException("Отступ от края слишком велик для такого стола")
    }

    companion object {
        const val DEFAULT_MARGIN_MM = 5.0
        private val SIZE_RANGE_MM = 50.0..1000.0
        private val MARGIN_RANGE_MM = 0.0..30.0
    }
}
