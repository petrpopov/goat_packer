package com.goatpacker.web.job

/**
 * Перевод известных ошибок Rust-бинарников в человекопонятные заголовки. Работает по хвосту
 * log.txt джобы: бинарники пишут диагностику в поток вывода, и без перевока пользователь
 * видит сырые английские строки вроде "source printer 'Bambu Lab P1S' does not match
 * profile 'Snapmaker U1'". Вернувшийся null = шаблон не опознан, покажем хвост как есть.
 *
 * Заголовок показывается в UI первым, сырой хвост — под ним после разделителя
 * [DETAILS_SEPARATOR] (фронт разворачивает его в свёрнутый блок).
 */
object CliErrorTranslator {
    /** Разделитель заголовка и сырых деталей; часть контракта с фронтендом. */
    const val DETAILS_SEPARATOR = "\n— детали —\n"

    private data class Pattern(val regex: Regex, val message: (MatchResult) -> String)

    private val PATTERNS = listOf(
        Pattern(Regex("must contain exactly one painted object; found (\\d+)")) {
            "В файле несколько объектов (${it.groupValues[1]}): упаковка рассчитана на проект с одним объектом на одном листе. Удалите лишние копии в слайсере и сохраните проект заново."
        },
        Pattern(Regex("source printer '([^']+)' does not match profile '([^']+)'")) {
            "Принтер в файле — «${it.groupValues[1]}», а выбран профиль «${it.groupValues[2]}». Загрузите файл, соответствующий профилю."
        },
        Pattern(Regex("too close to exclusion area")) {
            "Слайсер счёл объект слишком близким к служебной зоне стола (край стола или зона объезда). Попробуйте другой seed или меньшее число объектов."
        },
        Pattern(Regex("slicing failed \\(([^)]+)\\)\\n\\s*slicer report: (.+)")) {
            "Не удалось нарезать итоговую пластину (${it.groupValues[1]}): ${it.groupValues[2].trim()}"
        },
        Pattern(Regex("has no Metadata/project_settings\\.config entry")) {
            "Файл не похож на проект BambuStudio/Orca: в нём нет настроек проекта. Пересохраните модель как проект 3MF в слайсере."
        },
        Pattern(Regex("nozzle-clean injection failed: (.+)")) {
            "Не удалось встроить чистку сопла в итоговый файл: ${it.groupValues[1].trim()}"
        },
        Pattern(Regex("3MF assembly failed: (.+)")) {
            "Не удалось собрать итоговый 3MF-файл: ${it.groupValues[1].trim()}"
        },
        Pattern(Regex("even one object exceeds the time window: ([\\d.]+) h > window ([\\d.]+) h")) {
            "Даже один объект печатается дольше окна: ${it.groupValues[1]} ч (с коррекцией и запасом) при окне ${it.groupValues[2]} ч. Увеличьте окно или уменьшите запас."
        },
        Pattern(Regex("time window ([\\d.]+) h not met after (\\d+) window slices; last: (\\d+) objects = ([\\d.]+) h")) {
            "Не удалось уложиться в окно ${it.groupValues[1]} ч за ${it.groupValues[2]} пробных нарезки: последний вариант — ${it.groupValues[3]} шт. за ${it.groupValues[4]} ч. Увеличьте окно или уменьшите запас."
        },
        Pattern(Regex("prime tower changed again after the permitted re-pack")) {
            "Башня очистки выросла повторно даже после перепаковки — раскладка не стабилизировалась. Попробуйте другой seed."
        },
    )

    /** Заголовок для известной ошибки или null, если хвост не опознан. */
    fun translate(logTail: String): String? =
        PATTERNS.firstNotNullOfOrNull { pattern ->
            pattern.regex.find(logTail)?.let(pattern.message)
        }
}
