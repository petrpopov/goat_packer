package com.goatpacker.web.service.runner

import com.goatpacker.web.model.JobKind
import java.nio.file.Files
import java.nio.file.LinkOption
import java.nio.file.Path

/**
 * Имена файлов каталога джоба и white-list артефактов: какие имена разрешено отдавать клиенту и где лежат
 * соответствующие файлы. Перечисляются только реально существующие файлы.
 */
object JobArtifacts {
    const val LOG_FILE = "log.txt"
    const val PROFILE_FILE = "profile.json"
    const val PACK_WORK_DIR = "work"
    const val PACKED_3MF = "packed.3mf"

    /** Отчёт пакера: пишется рядом с `--output` как `packed.verification.json` (with_extension). */
    const val PACKED_REPORT = "packed.verification.json"

    /** Состояние упаковки без полной нарезки (`--no-final-slice`), как его пишет пакер. */
    const val PACKED_STATE = "packed.state.json"

    /** Публичное имя состояния упаковки (`packed.state.json` пакера). */
    const val PACK_STATE = "pack_state.json"

    /** Вход доводки — состояние родителя под своим именем: не артефакт pack_state.json самой доводки. */
    const val FINISH_STATE = "finish.state.json"

    const val SCHEDULE_CSV = "schedule.csv"
    const val SCHEDULE_XLSX = "schedule.xlsx"
    const val PLATES_3MF = "plates.3mf"
    const val SCHEDULE_REPORT = "report.json"

    /** Каталог вывода ams_calibration внутри каталога джоба (черновики — в out/slicing/, наружу не отдаются). */
    const val CALIBRATION_OUT_DIR = "out"
    const val CALIBRATION_A = "$CALIBRATION_OUT_DIR/calib_A.3mf"
    const val CALIBRATION_B = "$CALIBRATION_OUT_DIR/calib_B.3mf"
    const val CALIBRATION_INSTRUCTIONS = "$CALIBRATION_OUT_DIR/instructions.md"
    const val CALIBRATION_REPORT = "$CALIBRATION_OUT_DIR/calibration.json"

    /** Файлы, которые обязаны существовать после успешного запуска, иначе джоб считается упавшим. */
    val REQUIRED_PACK = listOf(PACKED_3MF)
    val REQUIRED_SCHEDULE = listOf(SCHEDULE_CSV, PLATES_3MF)
    val REQUIRED_CALIBRATION = listOf(CALIBRATION_A, CALIBRATION_B, CALIBRATION_INSTRUCTIONS, CALIBRATION_REPORT)

    /** Спецификация артефакта: публичное имя + способ найти файл в каталоге джоба. */
    data class ArtifactSpec(val name: String, val resolve: (Path) -> Path?)

    private fun firstExisting(dir: Path, vararg candidates: String): Path? =
        candidates.map { dir.resolve(it) }.firstOrNull {
            Files.isRegularFile(it, LinkOption.NOFOLLOW_LINKS) && it.toRealPath().startsWith(dir.toRealPath())
        }

    /** Артефакт [name], который лежит по одному из путей [candidates] каталога джоба. */
    private fun artifact(name: String, vararg candidates: String) = ArtifactSpec(name) { dir -> firstExisting(dir, *candidates) }

    fun whitelist(kind: JobKind): List<ArtifactSpec> = when (kind) {
        JobKind.PACK -> listOf(
            artifact(PACKED_3MF, PACKED_3MF),
            // Отчёт пакера наружу идёт под каноничным именем verification.json.
            artifact("verification.json", "verification.json", PACKED_REPORT),
            artifact("placements.json", "$PACK_WORK_DIR/placements.json", "placements.json"),
            // Состояние упаковки без полной нарезки: вход доводки «Нарезать стол».
            artifact(PACK_STATE, PACKED_STATE),
            artifact(LOG_FILE, LOG_FILE),
        )

        JobKind.SCHEDULE -> listOf(
            artifact(SCHEDULE_CSV, SCHEDULE_CSV),
            artifact(SCHEDULE_XLSX, SCHEDULE_XLSX),
            artifact(PLATES_3MF, PLATES_3MF),
            artifact(SCHEDULE_REPORT, SCHEDULE_REPORT),
            artifact(LOG_FILE, LOG_FILE),
        )

        // ams_calibration пишет в --out-dir out; черновики слайсинга (out/slicing/) не отдаются.
        JobKind.CALIBRATION -> listOf(
            artifact("calib_A.3mf", CALIBRATION_A),
            artifact("calib_B.3mf", CALIBRATION_B),
            artifact("instructions.md", CALIBRATION_INSTRUCTIONS),
            artifact("calibration.json", CALIBRATION_REPORT),
            artifact(LOG_FILE, LOG_FILE),
        )
    }

    /** Путь к артефакту по имени; null, если имя не в white-list или файла нет. */
    fun resolve(kind: JobKind, dir: Path, name: String): Path? =
        whitelist(kind).firstOrNull { it.name == name }?.resolve(dir)
            ?.takeIf { Files.isRegularFile(it, LinkOption.NOFOLLOW_LINKS) }

    /** Существующие артефакты: пары (публичное имя, путь). */
    fun listExisting(kind: JobKind, dir: Path): List<Pair<String, Path>> =
        whitelist(kind).mapNotNull { spec -> spec.resolve(dir)?.let { spec.name to it } }
}
