package com.goatpacker.web.job

import org.slf4j.LoggerFactory
import org.springframework.stereotype.Component
import java.util.concurrent.ConcurrentHashMap

/** In-memory реестр джобов. Без БД: джобы живут в памяти, файлы — в каталоге {jobs-root}. */
@Component
@org.springframework.context.annotation.Profile("!server")
class JobStore {
    companion object {
        private val log = LoggerFactory.getLogger(JobStore::class.java)
    }

    private val jobs = ConcurrentHashMap<String, Job>()

    fun put(job: Job) {
        jobs[job.id] = job
    }

    fun get(id: String): Job? = jobs[id]

    fun remove(id: String): Job? = jobs.remove(id)

    fun all(): Collection<Job> = jobs.values

    fun count(): Int = jobs.size

    @jakarta.annotation.PreDestroy
    fun shutdown() {
        jobs.values.forEach { job -> job.cancel()?.let { JobProcessKiller.kill(it) } }
    }
}
