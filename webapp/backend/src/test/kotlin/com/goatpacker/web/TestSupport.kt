package com.goatpacker.web

import com.fasterxml.jackson.databind.JsonNode
import com.fasterxml.jackson.databind.ObjectMapper
import com.jayway.jsonpath.JsonPath
import java.nio.file.Files
import java.nio.file.Path
import org.springframework.test.web.servlet.MockMvc
import org.springframework.test.web.servlet.request.MockMvcRequestBuilders.get

/** Установка фейковых CLI-бинарников во временный каталог (с битом исполнения). */
object FakeBins {

    /**
     * Gradle-ресурсы теряют бит исполнения, поэтому скрипт копируется из classpath во временный
     * каталог и chmod'ится явно.
     */
    fun install(root: Path, resourceName: String, targetName: String = resourceName): String {
        val bytes = requireNotNull(FakeBins::class.java.getResourceAsStream("/bin/$resourceName")) {
            "нет ресурса /bin/$resourceName"
        }.readBytes()
        val target = root.resolve(targetName)
        Files.createDirectories(target.toAbsolutePath().parent)
        Files.write(target, bytes)
        target.toFile().setReadable(true, false)
        target.toFile().setExecutable(true, false)

        return target.toAbsolutePath().toString()
    }
}

/** Поллинг статуса джоба до терминального состояния (таймаут 30 с). */
fun MockMvc.awaitJobCompletion(
    id: String,
    timeoutMs: Long = 30_000,
    mapper: ObjectMapper = ObjectMapper(),
): JsonNode {
    val deadline = System.currentTimeMillis() + timeoutMs
    var lastBody = ""
    while (System.currentTimeMillis() < deadline) {
        lastBody = perform(get("/api/jobs/$id"))
            .andExpect(org.springframework.test.web.servlet.result.MockMvcResultMatchers.status().isOk)
            .andReturn()
            .response
            .contentAsString
        when (val status = JsonPath.read<String>(lastBody, "$.status")) {
            "SUCCEEDED" -> return mapper.readTree(lastBody)
            "FAILED" -> throw AssertionError("Джоба $id провалилась: $lastBody")
        }
        Thread.sleep(150)
    }
    throw AssertionError("Джоба $id не завершилась за $timeoutMs мс; последний статус: $lastBody")
}
