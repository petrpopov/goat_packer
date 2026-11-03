package com.goatpacker.web.service.runner

import org.springframework.context.annotation.Profile
import org.springframework.stereotype.Component
import java.util.concurrent.CompletableFuture
import java.util.concurrent.TimeUnit

/** Вызовы консольного `docker`: выполняет команду с ограничением времени и вывода, снимает контейнеры. */
@Component
@Profile("server")
class DockerCli {
    /** Выполняет [args] и возвращает вывод (не больше [MAX_OUTPUT_BYTES]); ненулевой код или зависание — исключение. */
    fun run(args: List<String>): String {
        val process = ProcessBuilder(args).redirectErrorStream(true).start()
        val output = CompletableFuture.supplyAsync { process.inputStream.readNBytes(MAX_OUTPUT_BYTES).toString(Charsets.UTF_8) }
        if (!process.waitFor(COMMAND_TIMEOUT_SECONDS, TimeUnit.SECONDS)) {
            process.destroyForcibly()
            error("Runtime command timed out")
        }
        val text = output.get(OUTPUT_DRAIN_SECONDS, TimeUnit.SECONDS)
        check(process.exitValue() == 0) { "Runtime error: ${text.take(MAX_ERROR_CHARS)}" }
        return text
    }

    /** `docker rm -f`; если команда не прошла, проверяет, что контейнера действительно нет, иначе исключение. */
    fun forceRemove(idOrName: String) {
        val process = ProcessBuilder("docker", "rm", "-f", idOrName)
            .redirectOutput(ProcessBuilder.Redirect.DISCARD)
            .redirectError(ProcessBuilder.Redirect.DISCARD)
            .start()
        if (!process.waitFor(REMOVE_TIMEOUT_SECONDS, TimeUnit.SECONDS)) {
            process.destroyForcibly()
            error("Docker termination timed out")
        }
        if (process.exitValue() != 0) {
            check(awaitGone(idOrName)) { "Native container termination not confirmed" }
        }
    }

    /**
     * `docker rm -f` also fails while a `--rm` container is already being removed (the export
     * helper right after its run): it is still listed for a moment. Production v0.6.0 failed a
     * finished schedule job this way once the probe-cache export added one more helper run.
     */
    private fun awaitGone(idOrName: String): Boolean {
        repeat(REMOVAL_POLLS) {
            if (!containerExists(idOrName)) return true
            Thread.sleep(REMOVAL_POLL_MILLIS)
        }
        return !containerExists(idOrName)
    }

    private fun containerExists(idOrName: String): Boolean =
        run(listOf("docker", "ps", "-a", "--format", "{{.ID}} {{.Names}}")).lines()
            .filter(String::isNotBlank)
            .any { idOrName.startsWith(it.substringBefore(' ')) || it.substringAfter(' ') == idOrName }

    private companion object {
        const val COMMAND_TIMEOUT_SECONDS = 60L
        const val OUTPUT_DRAIN_SECONDS = 5L
        const val REMOVE_TIMEOUT_SECONDS = 15L
        const val REMOVAL_POLLS = 40
        const val REMOVAL_POLL_MILLIS = 250L
        const val MAX_OUTPUT_BYTES = 65_536
        const val MAX_ERROR_CHARS = 1000
    }
}
