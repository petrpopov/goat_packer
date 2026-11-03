package com.goatpacker.web.service

import com.fasterxml.jackson.databind.JsonNode
import com.fasterxml.jackson.databind.node.ObjectNode
import com.goatpacker.web.config.SlicerKind
import com.goatpacker.web.dto.ProfileDto
import com.goatpacker.web.dto.SlicerOptionDto
import com.goatpacker.web.exception.badRequest

/**
 * Слайсер, в котором пользователь будет печатать STL-задачу (BambuStudio или OrcaSlicer). Профиль
 * перечисляет допустимые в `slicers` (вид → GUI, которым открывают стол; пишет printer_catalog
 * slicers), умолчание — его slicer_kind: Bambu для принтеров Bambu Lab, Orca для остальных (у
 * Snapmaker — Snapmaker Orca, сервер режет ванильным Orca). Выбор записывается в снимок профиля
 * задачи: по slicer_kind воркер берёт runner-образ и локальный слайсер, по gui_flavour
 * source_project штампует проект для этого GUI; пакеру уходит `--slicer`. Проверка нарезкой
 * копирует снимок родителя, поэтому слайсер у неё тот же.
 */
object SlicerChoice {
    const val ONLY_FOR_STL = "Слайсер выбирается только для STL: проект 3MF режется тем слайсером, для которого сохранён"

    /** Слайсеры профиля по json: `slicers` (вид → GUI), умолчание первым; без поля — только slicer_kind с gui_flavour. */
    fun offered(node: JsonNode): List<SlicerOptionDto> {
        val own = node.path("slicer_kind").asText("")
        val listed = node.path("slicers")
        val kinds = listOf(own) + listed.fieldNames().asSequence().filter { it != own }.sorted().toList()
        return kinds.map { kind ->
            val gui = listed.path(kind).asText("").ifBlank { if (kind == own) node.path("gui_flavour").asText("") else "" }
            SlicerOptionDto(kind, gui)
        }
    }

    /**
     * Слайсер задачи: для STL — [requested] или умолчание профиля (всегда явный, пакер и
     * source_project получают его как есть); для 3MF — null, а заданный [requested], отличный от
     * умолчания, — 400.
     */
    fun resolve(profile: ProfileDto, requested: String?, isStl: Boolean): String? {
        val wanted = requested?.trim()?.takeIf { it.isNotEmpty() }
        if (!isStl) {
            if (wanted != null && wanted != profile.slicerKind) badRequest(ONLY_FOR_STL)
            return null
        }
        val kind = wanted ?: profile.slicerKind
        if (profile.slicers.none { it.kind == kind }) {
            badRequest("Принтер «${profile.name}» нельзя нарезать в ${label(kind)}: доступно ${profile.slicers.joinToString(", ") { label(it.kind) }}")
        }
        return kind
    }

    /** Снимок профиля под слайсер [kind]: slicer_kind, путь runner-образа и GUI из `slicers`. true — изменён. */
    fun apply(profile: ObjectNode, kind: String): Boolean {
        if (profile.path("slicer_kind").asText() == kind) return false
        profile.put("slicer_kind", kind)
        profile.put("slicer_path", LocalSlicer.runnerSlicerPath(kind))
        profile.path("slicers").path(kind).asText("").takeIf { it.isNotBlank() }?.let { profile.put("gui_flavour", it) }
        return true
    }

    private fun label(kind: String) = if (kind == SlicerKind.BAMBU) "Bambu Studio" else "OrcaSlicer"
}
