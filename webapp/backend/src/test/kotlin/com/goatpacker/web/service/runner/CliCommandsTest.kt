package com.goatpacker.web.service.runner

import com.goatpacker.web.model.PackParams
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertFalse
import org.junit.jupiter.api.Test
import java.nio.file.Path

class CliCommandsTest {
    private val base = PackParams(
        profileId = "test",
        profilePath = Path.of("profile.json"),
        clearance = 0.7,
        attempts = 1,
        edgeMargin = null,
        layout = "dense",
        nozzleClean = "",
    )

    @Test
    fun `раскладка передаётся как dense или grid, прежнее search — это dense`() {
        val layout = { params: PackParams ->
            val args = CliCommands.pack("packer", params)
            args[args.indexOf("--layout") + 1]
        }
        assertEquals("dense", layout(base))
        assertEquals("grid", layout(base.copy(layout = "grid")))
        assertEquals("dense", layout(base.copy(layout = "search")))
    }

    @Test
    fun `флаги совместимости, ни на что не влиявшие, пакеру не передаются`() {
        val args = CliCommands.pack("packer", base)
        for (gone in listOf("--candidates", "--seed", "--stall", "--angle-step", "--no-final-slice")) {
            assertFalse(gone in args, gone)
        }
    }

    @Test
    fun `ширина башни передаётся пакеру только заданной`() {
        val args = CliCommands.pack("packer", base.copy(towerWidth = 12.5))
        assertEquals("12.5", args[args.indexOf("--tower-width") + 1])
        // Пусто = дефолт пакера (20 мм): флаг не передаётся.
        assertFalse("--tower-width" in CliCommands.pack("packer", base))
    }

    @Test
    fun `растр не передаётся - пакер идёт своей лестницей`() {
        assertFalse("--resolution" in CliCommands.pack("packer", base))
    }

    @Test
    fun `быстрый расчёт поддержек передаётся флагом, точный — дефолт пакера`() {
        val args = CliCommands.pack("packer", base.copy(precision = "fast"))
        assertEquals("fast", args[args.indexOf("--precision") + 1])
        assertFalse("--precision" in CliCommands.pack("packer", base))
    }
}
