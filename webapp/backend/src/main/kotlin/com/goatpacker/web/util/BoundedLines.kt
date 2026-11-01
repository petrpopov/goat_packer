package com.goatpacker.web.util

import java.io.InputStream

/** Построчное чтение вывода процесса с ограничением длины строки. */
object BoundedLines {
    /**
     * 64 КБ: layout-строка пакера стремится к ≤ 16 КБ (контур упрощается), но при сотнях объектов одни
     * placements могут её превысить; строка сверх лимита обрезается и потом игнорируется как
     * неразбираемая — это безвредно. Предел бережёт память.
     */
    const val DEFAULT_LIMIT = 65_536

    private const val READ_CHUNK = 4096
    private const val TRUNCATED_SUFFIX = " [truncated]"

    fun read(input: InputStream, limit: Int = DEFAULT_LIMIT, consume: (String) -> Unit) {
        input.reader(Charsets.UTF_8).use { reader ->
            val buffer = CharArray(READ_CHUNK)
            val line = StringBuilder()
            var truncated = false

            fun flush() {
                consume(line.toString() + if (truncated) TRUNCATED_SUFFIX else "")
                line.setLength(0)
                truncated = false
            }

            while (true) {
                val count = reader.read(buffer)
                if (count < 0) break
                for (i in 0 until count) {
                    when (val char = buffer[i]) {
                        '\n' -> flush()
                        '\r' -> Unit
                        else -> if (line.length < limit) line.append(char) else truncated = true
                    }
                }
            }
            if (line.isNotEmpty() || truncated) flush()
        }
    }
}
