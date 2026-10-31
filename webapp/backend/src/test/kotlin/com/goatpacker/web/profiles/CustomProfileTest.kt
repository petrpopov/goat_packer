package com.goatpacker.web.profiles

import com.fasterxml.jackson.databind.ObjectMapper
import com.goatpacker.web.dto.CustomBed
import com.goatpacker.web.exception.BadRequestException
import com.goatpacker.web.service.CustomProfile
import com.goatpacker.web.service.ProjectBed
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertFalse
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test
import org.junit.jupiter.api.assertThrows

class CustomProfileTest {
    private val mapper = ObjectMapper()

    @Test
    fun `шаблон своего стола — формат PrinterProfile с выключенным неизвестным`() {
        val built = CustomProfile.build(mapper, CustomBed(220.0, 230.0, 250.0, 6.0), null, "/opt/orca")
        val p = mapper.readTree(built.json)
        assertEquals("custom", p["id"].asText()); assertEquals("Custom Printer", p["printer_name"].asText())
        assertEquals("orca", p["slicer_kind"].asText()); assertEquals("/opt/orca", p["slicer_path"].asText())
        assertEquals(listOf(0.0, 0.0, 220.0, 230.0), p["printable_area"].map { it.asDouble() })
        assertEquals(6.0, p["edge_margin_mm"].asDouble()); assertEquals(250.0, p["printable_height"].asDouble())
        assertTrue(p["exclusion_rectangles"].isEmpty && p["purge_lines"].isEmpty)
        assertEquals("wipe_tower", p["tower_marker_style"].asText()); assertEquals(0.5, p["tower_line_width_mm"].asDouble())
        assertFalse(p["blank_start_gcode_for_cli"].asBoolean()); assertTrue(p["no_check_for_final_tower_probe"].asBoolean())
        assertFalse(p.has("nozzle_clean_gcode") || p.has("filament_change_seconds"))
        assertEquals(220.0, built.dto.bedWidthMm); assertFalse(built.dto.supportsNozzleClean)
    }

    @Test
    fun `стол из проекта берётся, только если размеры подтверждены без правки`() {
        val project = ProjectBed(listOf(-10.0, 5.0, 240.0, 215.0), listOf(listOf(-10.0, 5.0, 10.0, 25.0)), 220.0)
        val same = mapper.readTree(CustomProfile.build(mapper, CustomBed(250.0, 210.0, 220.0), project, "o").json)
        assertEquals(listOf(-10.0, 5.0, 240.0, 215.0), same["printable_area"].map { it.asDouble() })
        assertEquals(1, same["exclusion_rectangles"].size())
        val edited = mapper.readTree(CustomProfile.build(mapper, CustomBed(300.0, 210.0, 220.0), project, "o").json)
        assertEquals(listOf(0.0, 0.0, 300.0, 210.0), edited["printable_area"].map { it.asDouble() })
        assertTrue(edited["exclusion_rectangles"].isEmpty)
    }

    @Test
    fun `границы своего стола`() {
        CustomBed(50.0, 1000.0, 50.0, 0.0).validate(); CustomBed(200.0, 200.0, 200.0, 30.0).validate()
        for (bad in listOf(CustomBed(49.0, 200.0, 200.0), CustomBed(200.0, 1001.0, 200.0), CustomBed(200.0, 200.0, Double.NaN),
            CustomBed(200.0, 200.0, 200.0, -1.0), CustomBed(200.0, 200.0, 200.0, 31.0), CustomBed(50.0, 50.0, 50.0, 25.0)))
            assertThrows<BadRequestException>(bad.toString()) { bad.validate() }
    }
}
