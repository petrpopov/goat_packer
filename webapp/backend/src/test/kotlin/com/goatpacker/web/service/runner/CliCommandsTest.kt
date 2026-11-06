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
        resolution = 0.25,
        attempts = 1,
        candidates = 1,
        seed = 1,
        edgeMargin = null,
        layout = "search",
        nozzleClean = "",
    )

    @Test
    fun `ширина башни передаётся пакеру только заданной`() {
        val args = CliCommands.pack("packer", base.copy(towerWidth = 12.5))
        assertEquals("12.5", args[args.indexOf("--tower-width") + 1])
        // Пусто = дефолт пакера (20 мм): флаг не передаётся.
        assertFalse("--tower-width" in CliCommands.pack("packer", base))
    }

    @Test
    fun `разрешение по умолчанию оставляет пакеру лестницу, заданное передаётся`() {
        assertFalse("--resolution" in CliCommands.pack("packer", base))
        val args = CliCommands.pack("packer", base.copy(resolution = 0.2))
        assertEquals("0.2", args[args.indexOf("--resolution") + 1])
    }
}
