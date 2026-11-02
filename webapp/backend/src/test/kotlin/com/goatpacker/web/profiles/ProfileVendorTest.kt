package com.goatpacker.web.profiles

import com.fasterxml.jackson.databind.ObjectMapper
import com.goatpacker.web.config.AppProperties
import com.goatpacker.web.service.ProfileService
import com.goatpacker.web.service.Project3mf
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Test
import org.junit.jupiter.api.io.TempDir
import java.nio.file.Files
import java.nio.file.Path

/** vendor профиля: явное поле важнее правила «printer_name без последнего слова». */
class ProfileVendorTest {
    private fun service(dir: Path): ProfileService {
        val props = AppProperties(profilesDir = dir.toString()).apply { resolveAbsolutePaths() }
        val mapper = ObjectMapper()
        return ProfileService(props, mapper, Project3mf(mapper))
    }

    private fun write(dir: Path, id: String, name: String, vendor: String?, display: String? = null) {
        val vendorField = (vendor?.let { ""","vendor":"$it"""" } ?: "") + (display?.let { ""","vendor_display":"$it"""" } ?: "")
        Files.writeString(dir.resolve("$id.json"),
            """{"id":"$id","printer_name":"$name"$vendorField,"slicer_kind":"orca","printable_area":[0,0,200,200]}""")
    }

    @Test
    fun `без поля vendor берётся printer_name без последнего слова, явное поле побеждает`(@TempDir dir: Path) {
        write(dir, "a", "Alpha Forge AF1 Max", null)
        write(dir, "b", "Beta 2.4 R2", "Beta Design")
        write(dir, "c", "Solo", null)
        // Каталог: vendor — папка пресетов OrcaSlicer, в выборе принтера — vendor_display.
        write(dir, "d", "Gamma Labs G2", "GLB", "Gamma Labs")
        val vendors = service(dir).list().associate { it.id to it.vendor }
        assertEquals(mapOf("a" to "Alpha Forge AF1", "b" to "Beta Design", "c" to "Solo", "d" to "Gamma Labs"), vendors)
    }
}
