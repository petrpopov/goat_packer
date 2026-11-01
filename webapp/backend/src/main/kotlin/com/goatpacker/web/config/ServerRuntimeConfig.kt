package com.goatpacker.web.config

import jakarta.annotation.PostConstruct
import org.springframework.boot.web.embedded.tomcat.TomcatConnectorCustomizer
import org.springframework.boot.web.embedded.tomcat.TomcatServletWebServerFactory
import org.springframework.boot.web.server.WebServerFactoryCustomizer
import org.springframework.context.annotation.Bean
import org.springframework.context.annotation.Configuration
import org.springframework.context.annotation.Profile

/** Проверка настроек серверного режима при старте и таймаут медленной передачи тела запроса. */
@Configuration
@Profile("server")
class ServerRuntimeConfig(private val props: PlatformProperties) {
    @PostConstruct
    fun validate() {
        require(props.workerCpus.isFinite() && props.workerCpus >= MIN_WORKER_CPUS && props.workerMemoryMib >= MIN_WORKER_MEMORY_MIB)
        require(props.executionCpuBudget.isFinite() && props.executionCpuBudget >= 0 && props.executionMemoryMib >= 0)
        require(props.powDifficulty in 1..MAX_POW_DIFFICULTY && props.maxRetries in 0..MAX_RETRIES)
        require(props.jobTimeoutSeconds in JOB_TIMEOUT_SECONDS && props.uploadTimeoutSeconds in UPLOAD_TIMEOUT_SECONDS)
        require(props.scratchLimitBytes > 0 && props.maxQueue in 1..MAX_QUEUE && props.storageLimitBytes > 0)
        if (props.production) {
            require(props.runnerMode == RunnerMode.DOCKER && !props.allowLocalRunner && props.storageMode == StorageMode.S3)
            require(props.requireChallenge) { "Production requires upload challenges and admission quotas" }
            require(props.workerMemoryMib * BYTES_PER_MIB > props.scratchLimitBytes + NATIVE_PROCESS_MEMORY_BYTES) {
                "Leave memory for native processes in addition to the charged tmpfs"
            }
        }
    }

    @Bean
    fun uploadSocketTimeout() = WebServerFactoryCustomizer<TomcatServletWebServerFactory> { factory ->
        factory.addConnectorCustomizers(
            TomcatConnectorCustomizer { connector ->
                connector.setProperty("disableUploadTimeout", "false")
                connector.setProperty("connectionUploadTimeout", UPLOAD_SOCKET_TIMEOUT_MS)
            },
        )
    }

    private companion object {
        const val MIN_WORKER_CPUS = 0.5
        const val MIN_WORKER_MEMORY_MIB = 512
        const val MAX_POW_DIFFICULTY = 22
        const val MAX_RETRIES = 3
        const val MAX_QUEUE = 1000
        const val BYTES_PER_MIB = 1024L * 1024
        const val NATIVE_PROCESS_MEMORY_BYTES = 512L * 1024 * 1024
        const val UPLOAD_SOCKET_TIMEOUT_MS = "30000"
        val JOB_TIMEOUT_SECONDS = 60..7200
        val UPLOAD_TIMEOUT_SECONDS = 30..600
    }
}
