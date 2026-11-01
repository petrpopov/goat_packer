package com.goatpacker.web.model

import com.goatpacker.web.dto.ProfileDto
import java.nio.file.Path

/**
 * Стол из project_settings.config 3MF неизвестного принтера: области в координатах стола, мм.
 * [printableArea] — [x0, y0, x1, y1]; [exclusionRectangles] — прямоугольники [x0, y0, x1, y1];
 * [printableHeight] — null, если в проекте нет (или не число).
 */
data class ProjectBed(
    val printableArea: List<Double>,
    val exclusionRectangles: List<List<Double>>,
    val printableHeight: Double?,
)

/** Результат разбора 3MF: есть ли модель, принтер из настроек и стол проекта (нужен, когда принтер не распознан). */
data class Inspection(val hasModel: Boolean, val printerModel: String?, val projectBed: ProjectBed? = null)

/** Профиль, разрешённый по id: DTO для проверок и путь к файлу для команд бинарников. */
data class ResolvedProfile(val dto: ProfileDto, val file: Path)

/**
 * Результат автоопределения профиля по загрузке. [printerModel] сохраняется и при неудаче
 * сопоставления — из него строится внятное «файл от принтера „X“, профиля для него нет»
 * вместо голого null.
 */
data class Detection(
    val profileId: String?,
    val printerModel: String?,
    val kind: String = UploadKind.PROJECT_3MF,
    val projectBed: ProjectBed? = null,
)

/** Вид загрузки в ответах API. */
object UploadKind {
    const val PROJECT_3MF = "3mf"
    const val MESH_STL = "stl"
}

/** Разовый профиль своего стола: DTO для статуса и json для `profile.json` джоба. */
data class BuiltProfile(val dto: ProfileDto, val json: String)
