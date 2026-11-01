package com.goatpacker.web.profiles

import com.fasterxml.jackson.databind.ObjectMapper
import com.goatpacker.web.dto.CustomBed
import com.goatpacker.web.exception.BadRequestException
import com.goatpacker.web.service.CustomProfile
import com.goatpacker.web.model.ProjectBed
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertFalse
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test
import org.junit.jupiter.api.assertThrows

class CustomProfileTest {
    private val mapper = ObjectMapper()

    @Test
    fun `шаблон своего стола — формат PrinterProfile с выключенным неизвестным`() {
        val built = CustomProfile.build(mapper, CustomBed(220.0, 230.0, 250.0, 6.0), null, "Prusa MK4") { "/opt/$it" }
        val p = mapper.readTree(built.json)
        assertEquals("custom", p["id"].asText()); assertEquals("Prusa MK4", p["printer_name"].asText())
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
    fun `стол 3MF — всегда стол проекта, размеры менять нельзя, отступ свой`() {
        val project = ProjectBed(listOf(-10.0, 5.0, 240.0, 215.0), listOf(listOf(-10.0, 5.0, 10.0, 25.0)), 220.0)
        // Поля формы округляют: расхождение до 0.5 мм — те же размеры.
        val same = mapper.readTree(CustomProfile.build(mapper, CustomBed(250.4, 209.6, 220.0, 8.0), project, null) { "o" }.json)
        assertEquals(listOf(-10.0, 5.0, 240.0, 215.0), same["printable_area"].map { it.asDouble() })
        assertEquals(1, same["exclusion_rectangles"].size())
        assertEquals(220.0, same["printable_height"].asDouble()); assertEquals(8.0, same["edge_margin_mm"].asDouble())
        for (edited in listOf(CustomBed(300.0, 210.0, 220.0), CustomBed(250.0, 211.0, 220.0), CustomBed(250.0, 210.0, 250.0))) {
            val error = assertThrows<BadRequestException>(edited.toString()) { CustomProfile.build(mapper, edited, project, null) { "o" } }
            assertEquals(CustomProfile.BED_FROM_PROJECT, error.message)
        }
        // Высоты в проекте нет — берётся введённая.
        val noHeight = mapper.readTree(CustomProfile.build(mapper, CustomBed(250.0, 210.0, 300.0), project.copy(printableHeight = null), null) { "o" }.json)
        assertEquals(300.0, noHeight["printable_height"].asDouble())
    }

    @Test
    fun `имя — printer_model проекта (пакер сверяет), слайсер — по модели как в Rust`() {
        val bambu = mapper.readTree(CustomProfile.build(mapper, CustomBed(256.0, 256.0, 256.0), null, "Bambu Lab X1 Carbon") { "/opt/$it" }.json)
        assertEquals("Bambu Lab X1 Carbon", bambu["printer_name"].asText())
        assertEquals("bambu", bambu["slicer_kind"].asText()); assertEquals("/opt/bambu", bambu["slicer_path"].asText())
        val none = mapper.readTree(CustomProfile.build(mapper, CustomBed(256.0, 256.0, 256.0), null, null) { "/opt/$it" }.json)
        assertEquals("Custom Printer", none["printer_name"].asText()); assertEquals("orca", none["slicer_kind"].asText())
    }

    @Test
    fun `границы своего стола`() {
        CustomBed(50.0, 1000.0, 50.0, 0.0).validate(); CustomBed(200.0, 200.0, 200.0, 30.0).validate()
        for (bad in listOf(CustomBed(49.0, 200.0, 200.0), CustomBed(200.0, 1001.0, 200.0), CustomBed(200.0, 200.0, Double.NaN),
            CustomBed(200.0, 200.0, 200.0, -1.0), CustomBed(200.0, 200.0, 200.0, 31.0), CustomBed(50.0, 50.0, 50.0, 25.0)))
            assertThrows<BadRequestException>(bad.toString()) { bad.validate() }
    }
}
