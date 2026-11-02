package com.goatpacker.web.service.runner

import com.goatpacker.web.service.runner.CliErrorTranslator
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertNull
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test

/**
 * Перевод известных ошибок бинарников в человекопонятные заголовки. Хвосты взяты из реальных
 * джоб: несоответствие принтера профилю (упавшая web-джоба 2026-09-19), отказ слайсера
 * «too close to exclusion area» (финальный слайс 17-объектной пластины).
 */
class CliErrorTranslatorTest {

    @Test
    fun `несколько объектов в файле переводятся с числом`() {
        val tail = "error: source project must contain exactly one painted object; found 46"
        val headline = CliErrorTranslator.translate(tail)!!
        assertTrue(headline.contains("46"), headline)
        assertTrue(headline.contains("одним объектом"), headline)
    }

    @Test
    fun `модель выше принтера — свой заголовок`() {
        val tail = "error: STL could not be converted: the model is taller than the printer: 230.0 mm, printable height 220.0 mm"
        assertEquals("Модель выше, чем позволяет принтер (230.0 mm, printable height 220.0 mm)", CliErrorTranslator.translate(tail))
    }

    @Test
    fun `ошибка подготовки проекта из STL переводится с причиной`() {
        val tail = "error: STL could not be converted: the model's footprint 300.0x20.0 mm does not fit the printable area 250.0x210.0 mm"
        val headline = CliErrorTranslator.translate(tail)!!
        assertTrue(headline.startsWith("Не удалось подготовить проект из STL: "), headline)
        assertTrue(headline.contains("300.0x20.0 mm"), headline)
    }

    @Test
    fun `несовпадение принтера и профиля называет обоих`() {
        val tail = "error: source printer 'Test Corner Bed' does not match profile 'Test Margin Bed'"
        val headline = CliErrorTranslator.translate(tail)!!
        assertTrue(headline.contains("Test Corner Bed"), headline)
        assertTrue(headline.contains("Test Margin Bed"), headline)
    }

    @Test
    fun `exclusion area от слайсера переводится`() {
        val tail = """
            error: Test Corner Bed slicing failed (exit status: 192)
            slicer report: goat4_magnet_x2.stl is too close to exclusion area, there may be collisions when printing.
        """.trimIndent()
        val headline = CliErrorTranslator.translate(tail)!!
        assertTrue(headline.contains("служебной зоне"), headline)
    }

    @Test
    fun `провал слайсинга выносит код и отчёт слайсера`() {
        val tail = """
            error: Test Corner Bed slicing failed (exit status: 154)
            slicer report: Some objects are too close; your G-code may have conflicts.
        """.trimIndent()
        val headline = CliErrorTranslator.translate(tail)!!
        assertTrue(headline.contains("154"), headline)
        assertTrue(headline.contains("Some objects are too close"), headline)
    }

    @Test
    fun `объект длиннее окна печати переводится с часами`() {
        val tail = "error: even one object exceeds the time window: 13.4 h > window 12.0 h"
        val headline = CliErrorTranslator.translate(tail)!!
        assertTrue(headline.contains("13.4") && headline.contains("12.0"), headline)
        assertTrue(headline.contains("Даже один объект"), headline)
    }

    @Test
    fun `ошибки переноса башни в центр переводятся`() {
        val keepout = CliErrorTranslator.translate(
            "error: relocated wipe tower would overlap a printer keep-out area; rerun with --no-centre",
        )!!
        assertTrue(keepout.contains("запретную зону"), keepout)
        val ignored = CliErrorTranslator.translate(
            "error: slicer ignored the relocated wipe tower (wipe_tower_x/y): final tower corner 170.0,180.0",
        )!!
        assertTrue(ignored.contains("не поставил башню"), ignored)
    }

    @Test
    fun `окно не достигнуто за отведённые нарезки переводится с последним вариантом`() {
        val tail = "error: time window 12.0 h not met after 4 window slices; last: 7 objects = 12.6 h corrected"
        val headline = CliErrorTranslator.translate(tail)!!
        assertTrue(headline.contains("12.0") && headline.contains("7 шт.") && headline.contains("12.6"), headline)
    }

    @Test
    fun `ошибка калибровки на русском отдаётся как есть`() {
        val tail = "error: для теста нужен проект минимум с двумя филаментами"
        assertEquals("для теста нужен проект минимум с двумя филаментами", CliErrorTranslator.translate(tail))
    }

    @Test
    fun `неопознанный хвост возвращает null без выдумок`() {
        assertNull(CliErrorTranslator.translate("something completely different went wrong"))
        assertNull(CliErrorTranslator.translate(""))
    }

    @Test
    fun `разделитель деталей стабилен для фронта`() {
        assertEquals("\n— детали —\n", CliErrorTranslator.DETAILS_SEPARATOR)
    }
}
