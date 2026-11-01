package com.goatpacker.web.service

import com.fasterxml.jackson.databind.ObjectMapper
import com.goatpacker.web.config.AppProperties
import com.goatpacker.web.dto.CustomBed
import com.goatpacker.web.exception.BadRequestException
import com.goatpacker.web.job.JobKind
import com.goatpacker.web.job.JobPhase
import com.goatpacker.web.job.ShellChain
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Test
import org.junit.jupiter.api.assertThrows

/** Сборка команды source_project и цепочки «конвертация && пакер». */
class StlInputsTest {
    private val mapper = ObjectMapper()

    @Test
    fun `каталожный принтер — vendor и orca_machine, свой стол — custom-bed из разового профиля`() {
        val catalog = mapper.readTree("""{"id":"prusa_mk4s","printer_name":"Prusa MK4S","vendor":"Prusa","vendor_display":"Prusa","orca_machine":"Prusa MK4S 0.4 nozzle"}""")
        assertEquals(
            listOf("/b/source_project", "--stl", "input.stl", "--presets", "/p", "--orca", "/s", "--vendor", "Prusa",
                "--machine", "Prusa MK4S 0.4 nozzle", "--output", "input.3mf", "--work-dir", "stl-work"),
            StlInputs.command("/b/source_project", "/p", "/s", catalog),
        )
        val custom = CustomProfile.build(mapper, CustomBed(220.5, 220.0, 250.0, 5.0), null, CustomProfile.NAME) { "/s" }
        assertEquals(listOf("--custom-bed", "220.5,220,250"), StlInputs.printerArgs(mapper.readTree(custom.json)))
        // Кромка профиля едет в source_project: модель должна влезть в стол за её вычетом.
        assertEquals(
            listOf("--custom-bed", "220.5,220,250", "--edge-margin", "5", "--output", "input.3mf"),
            StlInputs.command("/b", "/p", "/s", mapper.readTree(custom.json)).drop(7).take(6),
        )
    }

    @Test
    fun `проверка STL — бинарный по размеру, ASCII по solid и facet`() {
        fun binary(triangles: Int, extra: Int = 0): ByteArray = ByteArray(84 + 50 * triangles + extra).also {
            "solid but binary".toByteArray().copyInto(it)
            java.nio.ByteBuffer.wrap(it, 80, 4).order(java.nio.ByteOrder.LITTLE_ENDIAN).putInt(triangles)
        }
        fun valid(bytes: ByteArray) = StlInputs.looksLikeStl(bytes.size.toLong(), bytes.inputStream())
        assertEquals(true, valid(binary(2)))
        assertEquals(false, valid(binary(2, extra = 1)))
        assertEquals(true, valid("  solid cube\n facet normal 0 0 1\n".toByteArray()))
        assertEquals(false, valid("solid cube\nendsolid cube\n".toByteArray()))
        assertEquals(false, valid("PK\u0003\u0004 not an stl".toByteArray()))
        assertEquals(false, valid(ByteArray(0)))
        assertEquals("cat.3mf", StlInputs.projectName("cat.STL")); assertEquals("cat.3mf", StlInputs.projectName("cat.3mf"))
    }

    @Test
    fun `профиль без пресета машины из STL не собрать — 400`() {
        val error = assertThrows<BadRequestException> {
            StlInputs.printerArgs(mapper.readTree("""{"id":"x","printer_name":"Acme X","vendor":"Acme"}"""))
        }
        assertEquals("Для принтера «Acme X» проект из STL собрать нельзя — загрузите 3MF из слайсера", error.message)
    }

    @Test
    fun `локально без GOAT_SLICER_PRESETS STL не принимается, Bambu берёт своё дерево если оно задано`() {
        val orca = mapper.readTree("""{"slicer_kind":"orca"}""")
        val bambu = mapper.readTree("""{"slicer_kind":"bambu"}""")
        val empty = AppProperties()
        assertEquals(StlInputs.LOCAL_PRESETS_MISSING, assertThrows<BadRequestException> { StlInputs.localTools(empty, orca) }.message)
        val app = AppProperties(slicerPresets = "/orca/profiles", orcaSlicer = "/orca", bambuSlicer = "/bambu")
        assertEquals("/orca/profiles" to "/orca", StlInputs.localTools(app, bambu))
        assertEquals("/bambu/profiles" to "/bambu", StlInputs.localTools(app.copy(bambuPresets = "/bambu/profiles"), bambu))
    }

    @Test
    fun `цепочка — одна оболочка, кавычки экранированы, без подготовки команда как есть`() {
        val main = listOf("/bin/packer", "--source", "input.3mf")
        assertEquals(main, ShellChain.command(null, main))
        val chain = ShellChain.command(listOf("/bin/source_project", "--machine", "Snapmaker U1 (0.4 nozzle)", "--vendor", "O'Brien"), main)
        assertEquals(
            listOf("/bin/sh", "-c", "'/bin/source_project' '--machine' 'Snapmaker U1 (0.4 nozzle)' '--vendor' 'O'\\''Brien' && exec '/bin/packer' '--source' 'input.3mf'"),
            chain,
        )
        val run = ProcessBuilder(listOf("/bin/sh", "-c", ShellChain.script(listOf("printf", "%s|", "a b", "O'Brien"), listOf("echo", "ok")))).start()
        assertEquals("a b|O'Brien|ok", run.inputStream.readAllBytes().toString(Charsets.UTF_8).trim())
    }

    @Test
    fun `конвертация STL — сообщение и начало полосы пробы, пока пакер не начал этап`() {
        val tracker = ProgressTracker(JobKind.PACK, clock = { 0L }, initialMessage = StlInputs.CONVERSION_MESSAGE)
        var event = tracker.onLine("source project input.3mf: machine Prusa MK4S 0.4 nozzle, process p, filament f")
        assertEquals(JobPhase.PROBE, event.phase)
        assertEquals(PackStage.PROBE_SLICE.bandStart, event.progressPercent)
        assertEquals("Готовлю проект из STL…", event.phaseMessage)
        event = tracker.onLine("probing prime tower position (slicing 1 object)...")
        assertEquals("Пробная нарезка 1 объекта: ищу место башни очистки", event.phaseMessage)
    }
}
