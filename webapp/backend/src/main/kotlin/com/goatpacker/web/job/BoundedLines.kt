package com.goatpacker.web.job

import java.io.InputStream

object BoundedLines {
    fun read(input:InputStream,limit:Int=8192,consume:(String)->Unit) {
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
