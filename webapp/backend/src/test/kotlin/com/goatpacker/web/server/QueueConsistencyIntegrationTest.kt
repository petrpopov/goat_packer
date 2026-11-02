package com.goatpacker.web.server

import com.goatpacker.web.entity.ArtifactEntity
import com.goatpacker.web.entity.QueueJobEntity
import com.goatpacker.web.entity.QueueStatus
import com.goatpacker.web.entity.WorkerEntity
import com.goatpacker.web.entity.newId
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertFalse
import org.junit.jupiter.api.Assertions.assertNotNull
import org.junit.jupiter.api.Assertions.assertNull
import org.junit.jupiter.api.Assertions.assertThrows
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit

/** Целостность очереди под конкуренцией и сбоями: слоты, заборы (fence), подтверждение остановки, публикация. */
class QueueConsistencyIntegrationTest : ServerIntegrationTestBase() {
    private fun worker(id: String, bootId: String = newId(), capacity: Int = 1, memoryMib: Long = 1536) =
        WorkerEntity(id, bootId, 2.0, memoryMib * 1024 * 1024, capacity, 0)

    @Test fun `concurrent workers cannot exceed global slots or claim same job`() {
        val a = Client()
        a.guest()
        a.job(a.upload())
        val b = Client()
        b.guest()
        b.job(b.upload())
        (1..4).forEach { nodes.save(worker("worker-$it")) }
        nodes.save(worker("next-worker"))
        val executor = Executors.newFixedThreadPool(4)
        try {
            val claims = (1..4)
                .map { executor.submit<QueueJobEntity?> { execution.claim("worker-$it") } }
                .mapNotNull { it.get(10, TimeUnit.SECONDS) }
            assertEquals(1, claims.size)
            val job = claims.single()
            assertFalse(execution.complete(job.id, job.fence + 1, "{}", emptyList(), null))
            assertFalse(execution.complete(job.id, job.fence, null, emptyList(), "test failure"))
            assertEquals(QueueStatus.FAILED, jobs.findById(job.id).orElseThrow().status)
            assertNotNull(execution.claim("next-worker"))
        } finally {
            executor.shutdownNow()
        }
    }

    @Test fun `restart requires confirmed stop increments fence and limits retry budget`() {
        val c = Client()
        c.guest()
        val id = c.job(c.upload())["jobId"].asText()
        nodeService.beginRecovery("recover-node", "boot-a", "host-a")
        nodeService.pulse(worker("recover-node", "boot-a", memoryMib = 4096))
        val first = execution.claim("recover-node", "boot-a")!!
        assertEquals(1, first.attemptCount)
        execution.markRuntimeLost(id, first.fence, "simulated connection loss")
        assertNull(execution.claim("recover-node", "boot-a"))
        assertThrows(IllegalStateException::class.java) { nodeService.beginRecovery("recover-node", "boot-b", "another-host") }
        nodeService.beginRecovery("recover-node", "boot-b", "host-a")
        assertFalse(nodeService.pulse(worker("recover-node", "boot-a", memoryMib = 4096)))
        nodeService.confirmRecovered("recover-node", "boot-b")
        assertEquals(QueueStatus.RETRY_WAIT, jobs.findById(id).orElseThrow().status)
        assertFalse(execution.complete(id, first.fence, "{}", emptyList(), null))
        assertNull(execution.claim("recover-node", "boot-a"))
        jdbc.update("update queue_jobs set available_at=now() where id=?", id)
        nodeService.pulse(worker("recover-node", "boot-b", memoryMib = 4096))
        val second = execution.claim("recover-node", "boot-b")!!
        assertTrue(second.fence > first.fence)
        execution.retryStopped(id, second.fence, "second crash")
        assertEquals(QueueStatus.FAILED, jobs.findById(id).orElseThrow().status)
        assertEquals(0L, jdbc.queryForObject("select reserved_seconds from compute_budgets where owner_id='GLOBAL'", Long::class.java))
        assertEquals(2, jdbc.queryForObject("select count(*) from job_attempts where job_id=?", Int::class.java, id))
    }

    @Test fun `publication refuses expired staging but retains successfully committed objects`() {
        val c = Client()
        c.guest()
        val id = c.job(c.upload())["jobId"].asText()
        nodes.save(worker("publisher", memoryMib = 4096))
        val claim = execution.claim("publisher")!!
        val artifact = ArtifactEntity(jobId = id, name = "packed.3mf", downloadName = "cat_test_1_packed.3mf", objectKey = "test/staged", sizeBytes = 10)
        gc.stage(artifact.objectKey, 10)
        jdbc.update("update object_gc set deleting=true where object_key=?", artifact.objectKey)
        assertThrows(IllegalStateException::class.java) { execution.complete(id, claim.fence, "{}", listOf(artifact), null) }
        jdbc.update("update object_gc set deleting=false where object_key=?", artifact.objectKey)
        assertTrue(execution.complete(id, claim.fence, "{}", listOf(artifact), null))
        assertFalse(execution.complete(id, claim.fence, "{}", listOf(artifact), null))
        assertEquals(0, jdbc.queryForObject("select count(*) from object_gc where object_key=?", Int::class.java, artifact.objectKey))
        assertEquals(1, jdbc.queryForObject("select count(*) from artifacts where object_key=?", Int::class.java, artifact.objectKey))
    }
}
