package com.goatpacker.web.repository

import com.goatpacker.web.model.Job
import com.goatpacker.web.util.ProcessKiller
import jakarta.annotation.PreDestroy
import org.springframework.context.annotation.Profile
import org.springframework.stereotype.Repository
import java.util.concurrent.ConcurrentHashMap

/** In-memory реестр джобов локального режима. Без БД: джобы живут в памяти, файлы — в каталоге {jobs-root}. */
@Repository
@Profile("!server")
class JobStore {
    private val jobs = ConcurrentHashMap<String, Job>()

    fun put(job: Job) {
        jobs[job.id] = job
    }

    fun get(id: String): Job? = jobs[id]

    fun remove(id: String): Job? = jobs.remove(id)

    fun all(): Collection<Job> = jobs.values

    fun count(): Int = jobs.size

    /** При остановке приложения процессы бинарников не должны пережить его. */
    @PreDestroy
    fun killRunningProcesses() {
        jobs.values.forEach { job -> job.cancel()?.let(ProcessKiller::kill) }
    }
}
