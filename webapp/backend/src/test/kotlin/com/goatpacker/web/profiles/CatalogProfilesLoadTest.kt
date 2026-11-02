package com.goatpacker.web.profiles

import com.fasterxml.jackson.databind.ObjectMapper
import com.goatpacker.web.config.AppProperties
import com.goatpacker.web.service.ProfileService
import com.goatpacker.web.service.Project3mf
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test
import java.nio.file.Files
import java.nio.file.Path

/**
 * Единственная проверка, читающая настоящий каталог profiles/ репозитория: каждый его json
 * загружается ProfileService (битый профиль сервис молча пропускает с warn — здесь это провал),
 * id и printer_name уникальны (по printer_name идёт автоопределение принтера проекта).
 * Конкретные принтеры не называются: остальные тесты работают на src/test/resources/profiles.
 */
class CatalogProfilesLoadTest {
    private val catalog: Path = Path.of("../../profiles")

    @Test
    fun `весь каталог профилей загружается без пропусков, id и имена уникальны`() {
        val files = Files.list(catalog).use { s -> s.filter { it.fileName.toString().endsWith(".json") }.toList() }
        assertTrue(files.isNotEmpty(), "каталог $catalog пуст")
        val props = AppProperties(profilesDir = catalog.toString()).apply { resolveAbsolutePaths() }
        val mapper = ObjectMapper()
        val profiles = ProfileService(props, mapper, Project3mf(mapper)).list()

        assertEquals(files.size, profiles.size, "часть профилей не загрузилась (см. warn ProfileService)")
        assertEquals(profiles.size, profiles.map { it.id }.distinct().size, "повторяются id")
        val names = profiles.map { it.name }
        assertEquals(names.size, names.distinct().size, "повторяются printer_name: $names")
        profiles.forEach { p ->
            assertTrue(p.slicerKind in setOf("orca", "bambu"), "slicer_kind: $p")
            assertTrue(p.bedWidthMm > 0 && p.bedDepthMm > 0, "размер стола: $p")
            assertTrue(p.vendor.isNotBlank(), "производитель: $p")
        }
    }
}
