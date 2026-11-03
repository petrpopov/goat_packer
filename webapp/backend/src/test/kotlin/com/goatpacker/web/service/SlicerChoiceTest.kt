package com.goatpacker.web.service

import com.fasterxml.jackson.databind.ObjectMapper
import com.fasterxml.jackson.databind.node.ObjectNode
import com.goatpacker.web.dto.ProfileDto
import com.goatpacker.web.dto.SlicerOptionDto
import com.goatpacker.web.exception.BadRequestException
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertFalse
import org.junit.jupiter.api.Assertions.assertNull
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test
import org.junit.jupiter.api.assertThrows

/** Выбор слайсера STL-задачи: что предлагает профиль, проверка и снимок профиля под выбор. */
class SlicerChoiceTest {
    private val mapper = ObjectMapper()

    private val bambuPrinter = """{"slicer_kind":"bambu","slicer_path":"/opt/slicers/bambu/AppRun","gui_flavour":"bambu-02.08",
        "slicers":{"orca":"orca-2.4","bambu":"bambu-02.08"}}"""

    private fun dto(kind: String, slicers: List<String>) = ProfileDto(
        id = "p", name = "Test Printer", slicerKind = kind, bedWidthMm = 200.0, bedDepthMm = 200.0,
        edgeMarginMm = 0.0, supportsNozzleClean = false, slicers = slicers.map { SlicerOptionDto(it, "") },
    )

    @Test
    fun `профиль предлагает умолчание первым, без поля slicers — только его`() {
        assertEquals(
            listOf(SlicerOptionDto("bambu", "bambu-02.08"), SlicerOptionDto("orca", "orca-2.4")),
            SlicerChoice.offered(mapper.readTree(bambuPrinter)),
        )
        val snapmaker = mapper.readTree("""{"slicer_kind":"orca","gui_flavour":"snapmaker-orca-2.4"}""")
        assertEquals(listOf(SlicerOptionDto("orca", "snapmaker-orca-2.4")), SlicerChoice.offered(snapmaker))
        assertEquals(listOf(SlicerOptionDto("orca", "")), SlicerChoice.offered(mapper.readTree("""{"slicer_kind":"orca"}""")))
    }

    @Test
    fun `STL получает выбранный слайсер или умолчание, чужой — 400`() {
        val bambu = dto("bambu", listOf("bambu", "orca"))
        assertEquals("orca", SlicerChoice.resolve(bambu, "orca", isStl = true))
        assertEquals("bambu", SlicerChoice.resolve(bambu, null, isStl = true))
        assertEquals("bambu", SlicerChoice.resolve(bambu, " ", isStl = true))
        val error = assertThrows<BadRequestException> { SlicerChoice.resolve(dto("orca", listOf("orca")), "bambu", isStl = true) }
        assertEquals("Принтер «Test Printer» нельзя нарезать в Bambu Studio: доступно OrcaSlicer", error.message)
    }

    @Test
    fun `3MF режется своим слайсером — выбор другого 400, без выбора null`() {
        val bambu = dto("bambu", listOf("bambu", "orca"))
        assertNull(SlicerChoice.resolve(bambu, null, isStl = false))
        assertNull(SlicerChoice.resolve(bambu, "bambu", isStl = false))
        assertEquals(SlicerChoice.ONLY_FOR_STL, assertThrows<BadRequestException> { SlicerChoice.resolve(bambu, "orca", isStl = false) }.message)
    }

    @Test
    fun `снимок под другой слайсер — вид, путь runner-образа и GUI из slicers`() {
        val node = mapper.readTree(bambuPrinter) as ObjectNode
        assertFalse(SlicerChoice.apply(node, "bambu"))
        assertTrue(SlicerChoice.apply(node, "orca"))
        assertEquals("orca", node["slicer_kind"].asText())
        assertEquals("/opt/slicers/orca/AppRun", node["slicer_path"].asText())
        assertEquals("orca-2.4", node["gui_flavour"].asText())
    }
}
