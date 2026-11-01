package com.goatpacker.web.service.runner

import java.io.BufferedWriter
import java.io.Closeable
import java.nio.charset.StandardCharsets
import java.nio.file.Files
import java.nio.file.Path
import java.nio.file.StandardOpenOption
import java.util.ArrayDeque

/** Журнал вывода бинарника (log.txt) и разбор его хвоста для сообщения об ошибке. */
object JobLogs {
    /** Сколько последних строк лога идёт в error при FAILED. */
    private const val ERROR_TAIL_LINES = 15

    /** Длинные строки хвоста обрезаются: ошибка должна читаться, а не быть дампом. */
    private const val TAIL_LINE_MAX_CHARS = 4096

    /** JSON-событие пакера (stage, layout, slicer_progress ...): строка вида `{... "event" ...}`. */
    fun isEventLine(line: String): Boolean {
        val trimmed = line.trim()
        return trimmed.startsWith("{") && trimmed.contains("\"event\"")
    }

    fun isLayoutEvent(line: String): Boolean = line.contains("\"event\":\"layout\"")

    /** Событие слайсера (slicer_output/progress...): сыплются каждые пару секунд, в лог приложения идут на DEBUG. */
    fun isSlicerEvent(line: String): Boolean = line.contains("\"event\":\"slicer_")

    /**
     * Хвост лога с переведённым заголовком: известные ошибки бинарников получают человекопонятное «что случилось и
     * что делать», сырой хвост остаётся под разделителем (фронт показывает его свёрнутым). Неопознанный хвост
     * возвращается как есть. null — лога нет.
     */
    fun friendlyError(jobDir: Path, reason: String): String? {
        val tail = tail(jobDir) ?: return null
        val headline = CliErrorTranslator.translate(tail) ?: reason
        return headline + CliErrorTranslator.DETAILS_SEPARATOR + tail
    }

    private fun tail(jobDir: Path): String? {
        val logFile = jobDir.resolve(JobArtifacts.LOG_FILE)
        if (!Files.exists(logFile)) return null

        return try {
            val tail = ArrayDeque<String>(ERROR_TAIL_LINES)
            Files.newBufferedReader(logFile, StandardCharsets.UTF_8).useLines { lines ->
                lines.forEach { line ->
                    if (tail.size == ERROR_TAIL_LINES) tail.removeFirst()
                    // JSON-события (stage, slicer_progress) не показываем: ошибка должна объясняться человеческими строками.
                    if (isEventLine(line)) return@forEach
                    tail.addLast(line.takeLast(TAIL_LINE_MAX_CHARS))
                }
            }
            tail.joinToString("\n")
        } catch (_: Exception) {
            null
        }
    }
}

/** Дописывает строки вывода в log.txt, пока он не достиг [maxBytes]; layout-строки (до десятков КБ) не пишет вовсе. */
class JobLogWriter(file: Path, private val maxBytes: Long = DEFAULT_MAX_BYTES) : Closeable {
    private val writer: BufferedWriter = Files.newBufferedWriter(
        file,
        StandardCharsets.UTF_8,
        StandardOpenOption.CREATE,
        StandardOpenOption.APPEND,
    )
    private var written = 0L

    /** layout-строки съедали бы лимит, а в журнале не нужны. */
    fun append(line: String) {
        if (written >= maxBytes || JobLogs.isLayoutEvent(line)) return
        writer.write(line)
        writer.newLine()
        writer.flush()
        written += line.toByteArray(StandardCharsets.UTF_8).size + 1
    }

    override fun close() = writer.close()

    companion object {
        const val DEFAULT_MAX_BYTES = 10L * 1024 * 1024
    }
}
