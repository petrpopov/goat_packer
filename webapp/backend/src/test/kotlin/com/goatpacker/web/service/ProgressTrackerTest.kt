package com.goatpacker.web.service

import com.goatpacker.web.job.JobKind
import com.goatpacker.web.job.JobPhase
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Test

/** Юнит-тесты ProgressRules/ProgressTracker: все правила и монотонность. */
class ProgressTrackerTest {

    // ── Планировщик: чистые правила ────────────────────────────────────────────────────

    @Test
    fun `планировщик probe — N objects в конце строки`() {
        assertEquals(
            JobPhase.PROBE to 5,
            ProgressRules.parse(JobKind.SCHEDULE, "Bambu Lab A2L: 30 objects"),
        )
    }

    @Test
    fun `планировщик slicing sample — 10 плюс 65 умножить на i делить на j`() {
        assertEquals(
            JobPhase.BENCHMARK to 23, // 10 + 65 * 1 / 5
            ProgressRules.parse(JobKind.SCHEDULE, "[Bambu Lab A2L] slicing sample 1/5: batch 8/30"),
        )
        assertEquals(
            JobPhase.BENCHMARK to 75, // 10 + 65 * 5 / 5
            ProgressRules.parse(JobKind.SCHEDULE, "[Bambu Lab A2L] slicing sample 5/5: batch 30/30"),
        )
    }

    @Test
    fun `планировщик refining — небольшое продвижение от последнего j`() {
        assertEquals(
            JobPhase.BENCHMARK to 88, // 10 + 65 * (5+1) / 5
            ProgressRules.parse(JobKind.SCHEDULE, "[Bambu Lab A2L] refining: slicing batch 15 (2.1% curvature)", lastSampleTotal = 5),
        )
    }

    @Test
    fun `планировщик measured sizes — optimizing 78`() {
        assertEquals(
            JobPhase.OPTIMIZING to 78,
            ProgressRules.parse(JobKind.SCHEDULE, "[Bambu Lab A2L] 5 measured sizes cover batches 1..=30 (83% fewer slices than a full sweep)"),
        )
    }

    @Test
    fun `планировщик writing plate(s) — 90`() {
        assertEquals(
            JobPhase.WRITING to 90,
            ProgressRules.parse(JobKind.SCHEDULE, "Bambu Lab A2L: writing 4 plate(s) to plates.3mf"),
        )
    }

    @Test
    fun `планировщик summary objects и jobs — optimizing 95`() {
        assertEquals(
            JobPhase.OPTIMIZING to 95,
            ProgressRules.parse(
                JobKind.SCHEDULE,
                "Bambu Lab A2L: 63 objects, 8 jobs [7@10:00, 5@17:48], gain vs full plates: +12",
            ),
        )
    }

    // ── Состояние: монотонность и сообщения ────────────────────────────────────────────

    @Test
    fun `планировщик запоминает последний j для refining`() {
        val tracker = ProgressTracker(JobKind.SCHEDULE)
        tracker.onLine("[Bambu Lab A2L] slicing sample 5/5: batch 30/30") // 75
        val event = tracker.onLine("[Bambu Lab A2L] refining: slicing batch 15 (2.1% curvature)")

        assertEquals(JobPhase.BENCHMARK, event.phase)
        assertEquals(88, event.progressPercent)
    }

    @Test
    fun `последовательность планировщика — без откатов`() {
        val tracker = ProgressTracker(JobKind.SCHEDULE)
        assertEquals(JobPhase.PROBE, tracker.onLine("Bambu Lab A2L: 30 objects").phase)
        assertEquals(JobPhase.BENCHMARK, tracker.onLine("[Bambu Lab A2L] slicing sample 1/5: batch 8/30").phase)
        assertEquals(JobPhase.OPTIMIZING, tracker.onLine("[Bambu Lab A2L] 5 measured sizes cover batches 1..=30").phase)
        // Откат от 78 к bench-фазе отбрасывается.
        val rejected = tracker.onLine("[Bambu Lab A2L] slicing sample 2/5: batch 20/30")
        assertEquals(JobPhase.OPTIMIZING, rejected.phase)
        assertEquals(78, rejected.progressPercent)
        val writing = tracker.onLine("Bambu Lab A2L: writing 4 plate(s) to plates.3mf")
        assertEquals(JobPhase.WRITING to 90, writing.phase to writing.progressPercent)
        val summary = tracker.onLine("Bambu Lab A2L: 63 objects, 8 jobs [7@10:00], gain vs full plates: +12")
        assertEquals(JobPhase.OPTIMIZING to 95, summary.phase to summary.progressPercent)
    }

    @Test
    fun `калибровка стартует с нарезки и не откатывается к upload`() {
        val tracker = ProgressTracker(com.goatpacker.web.job.JobKind.CALIBRATION)
        val first = tracker.onLine("{")
        assertEquals(com.goatpacker.web.job.JobPhase.SLICING, first.phase)
        assertEquals(10, first.progressPercent)
        assertEquals(ProgressTracker.CALIBRATION_MESSAGE, first.phaseMessage)
        val done = tracker.onLine("  \"changes_sliced\": 10,")
        assertEquals(com.goatpacker.web.job.JobPhase.WRITING, done.phase)
        assertEquals(95, done.progressPercent)
    }
}
