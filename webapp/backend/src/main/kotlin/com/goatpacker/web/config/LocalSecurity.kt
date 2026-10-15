package com.goatpacker.web.config

import org.springframework.context.annotation.*
import org.springframework.security.config.annotation.web.builders.HttpSecurity
import org.springframework.security.web.SecurityFilterChain

/** Legacy local-only mode. Never publish this profile; server mode has mandatory auth/CSRF. */
@Configuration
@Profile("!server")
class LocalSecurity {
    @Bean fun localChain(http: HttpSecurity): SecurityFilterChain = http
        .csrf { it.disable() }.authorizeHttpRequests { it.anyRequest().permitAll() }.build()
}
