package com.goatpacker.web.job

import java.util.concurrent.TimeUnit

/** Завершение процессов бинарников (вместе с потомками — слайсерами). */
object JobProcessKiller {

    fun kill(process: Process) {
        try {
            process.descendants().forEach { it.destroy() }
            process.destroy()
            if (!process.waitFor(10, TimeUnit.SECONDS)) {
                process.descendants().forEach { it.destroyForcibly() }
                process.destroyForcibly()
            }
        } catch (_: InterruptedException) {
            Thread.currentThread().interrupt()
        }
    }
}
