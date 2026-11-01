package com.goatpacker.web.service

import com.fasterxml.jackson.databind.node.ObjectNode
import com.goatpacker.web.config.AppProperties

/**
 * Каталожные профили (printer_catalog) несут slicer_path runner-образа (/opt/slicers/<kind>/AppRun):
 * в docker-режиме воркер ставит путь образа, а локально (local-сервисы и серверный runnerMode=local)
 * такого пути нет — профиль джоба получает слайсер из настроек. Ручные профили (P1S и др. с путями
 * macOS) идут как есть.
 */
object LocalSlicer {
    const val RUNNER_PREFIX = "/opt/slicers/"
    private const val RUNNER_EXECUTABLE = "AppRun"

    /** Путь слайсера вида [slicerKind] внутри runner-образа. */
    fun runnerSlicerPath(slicerKind: String): String = "$RUNNER_PREFIX$slicerKind/$RUNNER_EXECUTABLE"

    /** Подменяет путь в [profile], если он из образа; true — профиль изменён. */
    fun localize(app: AppProperties, profile: ObjectNode): Boolean {
        if (!profile.path("slicer_path").asText("").startsWith(RUNNER_PREFIX)) return false
        profile.put("slicer_path", app.localSlicer(profile.path("slicer_kind").asText()))
        return true
    }
}
