package com.goatpacker.web.job

import java.io.InputStream

object BoundedLines {
    // 64 КБ: layout-строка пакера стремится к ≤ 16 КБ (контур упрощается), но при сотнях объектов одни placements могут её превысить; строка сверх лимита обрезается и потом игнорируется как неразбираемая — это безвредно. Предел бережёт память.
    fun read(input:InputStream,limit:Int=65536,consume:(String)->Unit) {
        input.reader(Charsets.UTF_8).use { reader ->
            val buffer=CharArray(4096);val line=StringBuilder();var truncated=false
            while(true) {
                val n=reader.read(buffer);if(n<0) break
                for(i in 0 until n) when(val c=buffer[i]) {
                    '\n' -> {consume(line.toString() + if(truncated) " [truncated]" else "");line.setLength(0);truncated=false}
                    '\r' -> {}
                    else -> if(line.length<limit) line.append(c) else truncated=true
                }
            }
            if(line.isNotEmpty() || truncated) consume(line.toString()+if(truncated) " [truncated]" else "")
        }
    }
}
