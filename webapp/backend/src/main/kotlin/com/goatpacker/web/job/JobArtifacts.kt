package com.goatpacker.web.job

import java.nio.file.Files
import java.nio.file.Path

/**
 * White-list артефактов джоба: какие имена разрешено отдавать клиенту и где лежат
 * соответствующие файлы в каталоге джоба. Перечисляются только реально существующие файлы.
 */
object JobArtifacts {

    const val LOG_FILE = "log.txt"

    /** Спецификация артефакта: публичное имя + способ найти файл в каталоге джоба. */
    data class ArtifactSpec(val name: String, val resolve: (Path) -> Path?)

    private fun firstExisting(dir: Path, vararg candidates: String): Path? =
        candidates.map { dir.resolve(it) }.firstOrNull {
            Files.isRegularFile(it, java.nio.file.LinkOption.NOFOLLOW_LINKS) && it.toRealPath().startsWith(dir.toRealPath())
        }

    fun whitelist(kind: JobKind): List<ArtifactSpec> = when (kind) {
        JobKind.PACK -> listOf(
            ArtifactSpec("packed.3mf") { dir -> firstExisting(dir, "packed.3mf") },
            // Пакер пишет отчёт рядом с --output как "packed.verification.json" (with_extension);
            // наружу он идёт под каноничным именем verification.json.
            ArtifactSpec("verification.json") { dir -> firstExisting(dir, "verification.json", "packed.verification.json") },
            ArtifactSpec("placements.json") { dir -> firstExisting(dir, "work/placements.json", "placements.json") },
            ArtifactSpec(LOG_FILE) { dir -> firstExisting(dir, LOG_FILE) },
        )

        JobKind.SCHEDULE -> listOf(
            ArtifactSpec("schedule.csv") { dir -> firstExisting(dir, "schedule.csv") },
            ArtifactSpec("schedule.xlsx") { dir -> firstExisting(dir, "schedule.xlsx") },
            ArtifactSpec("plates.3mf") { dir -> firstExisting(dir, "plates.3mf") },
            ArtifactSpec("report.json") { dir -> firstExisting(dir, "report.json") },
            ArtifactSpec(LOG_FILE) { dir -> firstExisting(dir, LOG_FILE) },
        )

        // ams_calibration пишет в --out-dir out; черновики слайсинга (out/slicing/) не отдаются.
        JobKind.CALIBRATION -> listOf(
            ArtifactSpec("calib_A.3mf") { dir -> firstExisting(dir, "out/calib_A.3mf") },
            ArtifactSpec("calib_B.3mf") { dir -> firstExisting(dir, "out/calib_B.3mf") },
            ArtifactSpec("instructions.md") { dir -> firstExisting(dir, "out/instructions.md") },
            ArtifactSpec("calibration.json") { dir -> firstExisting(dir, "out/calibration.json") },
            ArtifactSpec(LOG_FILE) { dir -> firstExisting(dir, LOG_FILE) },
        )
    }

    /** Путь к артефакту по имени; null, если имя не в white-list или файла нет. */
    fun resolve(kind: JobKind, dir: Path, name: String): Path? =
        whitelist(kind).firstOrNull { it.name == name }?.resolve(dir)?.takeIf { Files.isRegularFile(it, java.nio.file.LinkOption.NOFOLLOW_LINKS) }

    /** Существующие артефакты: пары (публичное имя, путь). */
    fun listExisting(kind: JobKind, dir: Path): List<Pair<String, Path>> =
        whitelist(kind).mapNotNull { spec -> spec.resolve(dir)?.let { spec.name to it } }
}
