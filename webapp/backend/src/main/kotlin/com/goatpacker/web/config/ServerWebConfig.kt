package com.goatpacker.web.config

import com.goatpacker.web.security.RateLimitInterceptor
import org.springframework.context.annotation.Configuration
import org.springframework.context.annotation.Profile
import org.springframework.web.servlet.config.annotation.InterceptorRegistry
import org.springframework.web.servlet.config.annotation.WebMvcConfigurer

/** MVC серверного режима: ограничитель частоты перед контроллерами `/api/v2`. */
@Configuration
@Profile("server")
class ServerWebConfig(private val rateLimits: RateLimitInterceptor) : WebMvcConfigurer {
    override fun addInterceptors(registry: InterceptorRegistry) {
        registry.addInterceptor(rateLimits).addPathPatterns("/api/v2/**")
    }
}
