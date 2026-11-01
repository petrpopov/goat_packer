package com.goatpacker.web.config

import org.springframework.context.annotation.Bean
import org.springframework.context.annotation.Configuration
import org.springframework.context.annotation.Profile
import org.springframework.scheduling.concurrent.ThreadPoolTaskExecutor
import java.util.concurrent.Executor
import java.util.concurrent.ThreadPoolExecutor

/** Имя бина исполнителя фоновых джоб локального режима (`@Async`). */
const val JOB_EXECUTOR = "jobExecutor"

/**
 * Исполнитель фоновых джоб. Упаковка (pack_footprint) выедает все ядра машины, поэтому джобы
 * выполняются строго по одному: core = max = 1. Очередь ограничена, при переполнении —
 * AbortPolicy, чтобы вызывающий сразу получил отказ, а не ждал в невидимой очереди.
 */
@Configuration
@Profile("!server")
class AsyncConfig {
    @Bean(name = [JOB_EXECUTOR])
    fun jobExecutor(): Executor = ThreadPoolTaskExecutor().apply {
        corePoolSize = 1
        maxPoolSize = 1
        queueCapacity = JOB_QUEUE_CAPACITY
        setThreadNamePrefix("job-")
        setRejectedExecutionHandler(ThreadPoolExecutor.AbortPolicy())
        initialize()
    }

    private companion object {
        const val JOB_QUEUE_CAPACITY = 32
    }
}
