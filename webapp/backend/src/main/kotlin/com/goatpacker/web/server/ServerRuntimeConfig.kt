package com.goatpacker.web.server

import jakarta.annotation.PostConstruct
import org.springframework.boot.web.embedded.tomcat.TomcatServletWebServerFactory
import org.springframework.boot.web.server.WebServerFactoryCustomizer
import org.springframework.context.annotation.*

@Configuration @Profile("server")
class ServerRuntimeConfig(private val props:PlatformProperties) {
    @PostConstruct fun validate() {
        require(props.workerCpus.isFinite() && props.workerCpus>=0.5 && props.workerMemoryMib>=512)
        require(props.executionCpuBudget.isFinite() && props.executionCpuBudget>=0 && props.executionMemoryMib>=0)
        require(props.powDifficulty in 1..22 && props.maxRetries in 0..3)
        require(props.jobTimeoutSeconds in 60..7200 && props.uploadTimeoutSeconds in 30..600)
        require(props.scratchLimitBytes>0 && props.maxQueue in 1..1000 && props.storageLimitBytes>0)
        if(props.production) {
            require(props.runnerMode=="docker" && !props.allowLocalRunner && props.storageMode=="s3")
            require(props.requireChallenge) {"Production requires upload challenges and admission quotas"}
            require(props.workerMemoryMib*1024*1024>props.scratchLimitBytes+512L*1024*1024) {"Leave memory for native processes in addition to the charged tmpfs"}
        }
    }
    @Bean fun uploadSocketTimeout()=WebServerFactoryCustomizer<TomcatServletWebServerFactory> { factory ->
        factory.addConnectorCustomizers(org.springframework.boot.web.embedded.tomcat.TomcatConnectorCustomizer { connector ->
            connector.setProperty("disableUploadTimeout","false")
            connector.setProperty("connectionUploadTimeout","30000")
        })
    }
}
