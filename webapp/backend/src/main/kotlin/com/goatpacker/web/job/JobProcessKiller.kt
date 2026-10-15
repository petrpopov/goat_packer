package com.goatpacker.web.job

import java.util.concurrent.TimeUnit

/** Snapshot children BEFORE killing the parent: reparented slicers disappear from descendants(). */
object JobProcessKiller {
    fun kill(process: Process) {
        val children = process.descendants().use { it.toList() }
        try {
            children.asReversed().forEach { it.destroy() }
            process.destroy()
            process.waitFor(2, TimeUnit.SECONDS)
        } catch (_: InterruptedException) {
            Thread.currentThread().interrupt()
        } finally {
            children.asReversed().filter { it.isAlive }.forEach { it.destroyForcibly() }
            if (process.isAlive) process.destroyForcibly()
        }
    }
}
