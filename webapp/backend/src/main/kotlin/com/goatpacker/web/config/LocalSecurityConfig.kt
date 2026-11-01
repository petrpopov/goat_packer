package com.goatpacker.web.config

import org.springframework.context.annotation.Bean
import org.springframework.context.annotation.Configuration
import org.springframework.context.annotation.Profile
import org.springframework.security.config.annotation.web.builders.HttpSecurity
import org.springframework.security.web.SecurityFilterChain

/** Legacy local-only mode. Never publish this profile; server mode has mandatory auth/CSRF. */
@Configuration
@Profile("!server")
class LocalSecurityConfig {
    @Bean
    fun localChain(http: HttpSecurity): SecurityFilterChain = http
        .csrf { it.disable() }
        .authorizeHttpRequests { it.anyRequest().permitAll() }
        .build()
}
