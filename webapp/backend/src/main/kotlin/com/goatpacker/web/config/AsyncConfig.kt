package com.goatpacker.web.config

import org.springframework.context.annotation.Bean
import org.springframework.context.annotation.Configuration
import org.springframework.scheduling.concurrent.ThreadPoolTaskExecutor
import java.util.concurrent.Executor
import java.util.concurrent.ThreadPoolExecutor

/**
 * Исполнитель фоновых джоб. Упаковка (pack_footprint) выедает все ядра машины, поэтому джобы
 * выполняются строго по одному: core = max = 1. Очередь ограничена (32), при переполнении —
 * AbortPolicy, чтобы вызывающий сразу получил отказ, а не ждал в невидимой очереди.
 */
@Configuration
class AsyncConfig {

    @Bean(name = ["jobExecutor"])
    fun jobExecutor(): Executor {
        val executor = ThreadPoolTaskExecutor()
        executor.corePoolSize = 1
        executor.maxPoolSize = 1
        executor.setQueueCapacity(32)
        executor.setThreadNamePrefix("job-")
        executor.setRejectedExecutionHandler(ThreadPoolExecutor.AbortPolicy())
        executor.initialize()

        return executor
    }
}
