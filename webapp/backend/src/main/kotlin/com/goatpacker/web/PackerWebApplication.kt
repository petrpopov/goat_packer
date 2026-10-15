package com.goatpacker.web

import org.springframework.boot.autoconfigure.SpringBootApplication
import org.springframework.boot.context.properties.ConfigurationPropertiesScan
import org.springframework.boot.runApplication
import org.springframework.scheduling.annotation.EnableAsync
import org.springframework.scheduling.annotation.EnableScheduling

@SpringBootApplication(exclude = [org.springframework.boot.autoconfigure.security.servlet.UserDetailsServiceAutoConfiguration::class])
@ConfigurationPropertiesScan
@EnableAsync
@EnableScheduling
class PackerWebApplication

fun main(args: Array<String>) {
    runApplication<PackerWebApplication>(*args)
}
