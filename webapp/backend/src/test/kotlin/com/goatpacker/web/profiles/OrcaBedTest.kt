package com.goatpacker.web.profiles

import com.fasterxml.jackson.databind.ObjectMapper
import com.goatpacker.web.service.OrcaBed
import com.goatpacker.web.service.ProjectBed
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertNull
import org.junit.jupiter.api.Test

/** Те же правила точек, что у orca_presets (Rust): список строк, одна строка, "0x0" = нет зоны. */
class OrcaBedTest {
    private val json = ObjectMapper()
    private fun bed(settings: String) = OrcaBed.fromSettings(json.readTree(settings))

    @Test
    fun `список точек, исключение-многоугольник становится габаритом, высота из строки`() {
        assertEquals(
            ProjectBed(listOf(0.0, 0.0, 256.0, 256.0), listOf(listOf(0.0, 0.0, 30.0, 20.0)), 250.0),
            bed("""{"printable_area":["0x0","256x0","256x256","0x256"],"bed_exclude_area":["0x0","30x0","0x20"],"printable_height":"250"}"""),
        )
    }

    @Test
    fun `одна строка через запятую и пустое исключение 0x0`() {
        assertEquals(
            ProjectBed(listOf(-5.0, 0.0, 300.0, 310.5), emptyList(), 400.0),
            bed("""{"printable_area":"-5x0,300x0,300x310.5,-5x310.5","bed_exclude_area":["0x0"],"printable_height":400}"""),
        )
    }

    @Test
    fun `круглый стол, нет области или мусор — стола нет`() {
        val circle = (0 until 32).joinToString(",") { i ->
            val a = i / 32.0 * 2 * Math.PI; "\"${100 * Math.cos(a)}x${100 * Math.sin(a)}\""
        }
        assertNull(bed("""{"printable_area":[$circle]}"""))
        assertNull(bed("""{"printer_model":"X"}"""))
        assertNull(bed("""{"printable_area":["0x0","abc"]}"""))
        assertNull(bed("""{"printable_area":["0x0","256x0","256x256","0x256"],"printable_height":"нет"}""")?.printableHeight)
    }
}
