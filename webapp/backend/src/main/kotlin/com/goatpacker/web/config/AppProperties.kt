package com.goatpacker.web.config

import jakarta.annotation.PostConstruct
import org.springframework.boot.context.properties.ConfigurationProperties
import org.springframework.stereotype.Component
import java.nio.file.Path
import java.nio.file.Paths

/**
 * Пути из application.yml разворачиваются в абсолютные при старте: относительные пути
 * резолвятся от рабочего каталога процесса. При запуске `./gradlew bootRun` из webapp/backend
 * рабочим каталогом является сам webapp/backend, поэтому дефолт `../../target/release/...`
 * указывает на бинарники в корне репозитория goat_packer.
 */
@Component
@ConfigurationProperties(prefix = "app")
data class AppProperties(
    var packerBin: String = "$RELEASE_DIR/colored_3mf_packer",
    var schedulerBin: String = "$RELEASE_DIR/daily_scheduler",
    var calibrationBin: String = "$RELEASE_DIR/ams_calibration",
    var profilesDir: String = "../../profiles",
    /** STL → 3MF перед упаковкой (Rust source_project). */
    var sourceProjectBin: String = "$RELEASE_DIR/source_project",
    /**
     * Дерево пресетов OrcaSlicer (`resources/profiles`, JSON) для STL в локальном режиме; пусто —
     * STL-джобы отклоняются. В docker-режиме пресеты берутся из образа слайсера.
     */
    var slicerPresets: String = "",
    /** Дерево пресетов Bambu Studio для STL на принтерах Bambu локально; пусто — экспорт Orca из [slicerPresets]. */
    var bambuPresets: String = "",
    /** Слайсеры для разового профиля своего стола (у каталожных профилей путь свой, в json). */
    var orcaSlicer: String = "/Applications/OrcaSlicer.app/Contents/MacOS/OrcaSlicer",
    var bambuSlicer: String = "/Applications/BambuStudio.app/Contents/MacOS/BambuStudio",
    var jobsRoot: String = "${System.getProperty("java.io.tmpdir")}/goat-packer-web/jobs",
    var jobTtlHours: Long = 12,
    /** Каталог кэша проб пакера для локального (десктопного) режима одного пользователя; пусто — без кэша. */
    var probeCacheDir: String = "",
    var allowedOrigins: List<String> = listOf("http://localhost:3007", "http://127.0.0.1:3007"),
) {
    lateinit var packerBinPath: Path
    lateinit var schedulerBinPath: Path
    lateinit var calibrationBinPath: Path
    lateinit var profilesDirPath: Path
    lateinit var sourceProjectBinPath: Path
    lateinit var jobsRootPath: Path

    @PostConstruct
    fun resolveAbsolutePaths() {
        packerBinPath = toAbsolute(packerBin)
        schedulerBinPath = toAbsolute(schedulerBin)
        calibrationBinPath = toAbsolute(calibrationBin)
        profilesDirPath = toAbsolute(profilesDir)
        sourceProjectBinPath = toAbsolute(sourceProjectBin)
        jobsRootPath = toAbsolute(jobsRoot)
    }

    private fun toAbsolute(raw: String): Path = Paths.get(raw).toAbsolutePath().normalize()

    /** Слайсер локального запуска для [slicerKind] (`bambu` или `orca`). */
    fun localSlicer(slicerKind: String): String = if (slicerKind == SlicerKind.BAMBU) bambuSlicer else orcaSlicer
}

private const val RELEASE_DIR = "../../target/release"

/** Виды слайсера в `slicer_kind` профилей. */
object SlicerKind {
    const val BAMBU = "bambu"
    const val ORCA = "orca"
    val ALL = setOf(BAMBU, ORCA)
}
