package com.goatpacker.web.service.local

import com.goatpacker.web.exception.BadRequestException
import com.goatpacker.web.exception.ErrorMessages
import com.goatpacker.web.model.Job
import com.goatpacker.web.repository.JobStore
import org.springframework.context.annotation.Profile
import org.springframework.stereotype.Service
import java.nio.file.Files
import java.util.concurrent.RejectedExecutionException

/**
 * Постановка джоба локального режима: каталог → подготовка файлов → регистрация → запуск. Любой сбой убирает
 * за собой следы (запись, процесс, каталог), а переполнение очереди исполнителя отдаётся понятной 400.
 */
@Service
@Profile("!server")
class LocalJobSubmitter(private val store: JobStore) {
    /** [prepare] кладёт входные файлы в каталог джоба, [start] запускает его (обычно `@Async`-метод раннера). */
    fun submit(job: Job, prepare: () -> Unit, start: () -> Unit) {
        Files.createDirectories(job.directory)
        try {
            prepare()
            store.put(job)
            start()
        } catch (ex: RejectedExecutionException) {
            discard(job)
            throw BadRequestException(ErrorMessages.QUEUE_FULL)
        } catch (ex: Exception) {
            discard(job)
            throw ex
        }
    }

    private fun discard(job: Job) {
        store.remove(job.id)
        job.cancel()
        job.directory.toFile().deleteRecursively()
    }
}
