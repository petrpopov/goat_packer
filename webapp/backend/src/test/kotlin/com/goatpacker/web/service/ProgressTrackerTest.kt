package com.goatpacker.web.service

import com.goatpacker.web.job.JobKind
import com.goatpacker.web.job.JobPhase
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Test

/** Юнит-тесты ProgressRules/ProgressTracker: все правила и монотонность. */
class ProgressTrackerTest {

    // ── Пакер: чистые правила ──────────────────────────────────────────────────────────

    @Test
    fun `пакер probe — 5 процентов`() {
        assertEquals(
            JobPhase.PROBE to 5,
            ProgressRules.parse(JobKind.PACK, "probing prime tower position (slicing 1 object)..."),
        )
    }

    @Test
    fun `пакер старт упаковки — 10 процентов`() {
        assertEquals(
            JobPhase.PACKING to 10,
            ProgressRules.parse(JobKind.PACK, "packing (2000 attempts, seed 20270831)..."),
        )
    }

    @Test
    fun `пакер pack_footprint — линейная формула`() {
        assertEquals(
            JobPhase.PACKING to 13, // 10 + 60 * 240 / 4800
            ProgressRules.parse(JobKind.PACK, "pack_footprint: 240/4800 attempts, best so far: 30 objects"),
        )
    }

    @Test
    fun `пакер pack_footprint — кламп при раннем стопе (X больше N)`() {
        assertEquals(
            JobPhase.PACKING to 100, // 10 + 60 * 2000 / 1000 = 130 → кламп 0..100
            ProgressRules.parse(JobKind.PACK, "pack_footprint: 2000/1000 attempts, best so far: 30 objects"),
        )
    }

    @Test
    fun `пакер best layout — assembling 72`() {
        assertEquals(
            JobPhase.ASSEMBLING to 72,
            ProgressRules.parse(JobKind.PACK, "pack_footprint: best layout: 30 objects (found at attempt 1800, 2000/2000 attempts run)"),
        )
    }

    @Test
    fun `пакер assembling 3MF — 75`() {
        assertEquals(
            JobPhase.ASSEMBLING to 75,
            ProgressRules.parse(JobKind.PACK, "packed 30 objects; assembling 3MF..."),
        )
    }

    @Test
    fun `пакер slicing final plate — 80`() {
        assertEquals(
            JobPhase.SLICING to 80,
            ProgressRules.parse(JobKind.PACK, "slicing final plate (30 objects)..."),
        )
    }

    @Test
    fun `пакер repack в любой форме — 40`() {
        assertEquals(
            JobPhase.REPACKING to 40,
            ProgressRules.parse(JobKind.PACK, "final tower outgrew probe keepout; repacking once"),
        )
        assertEquals(
            JobPhase.REPACKING to 40,
            ProgressRules.parse(JobKind.PACK, "re-packing (2000 attempts, seed 20270831)..."),
        )
    }

    @Test
    fun `пакер slicing repacked plate — это слайсинг, не repacking`() {
        assertEquals(
            JobPhase.SLICING to 80,
            ProgressRules.parse(JobKind.PACK, "slicing repacked plate (28 objects)..."),
        )
    }

    @Test
    fun `pack_footprint после начала repack — фаза repacking с прогрессом 40-70`() {
        // 40 + 30 * 480 / 4800 = 43
        assertEquals(
            JobPhase.REPACKING to 43,
            ProgressRules.parse(
                JobKind.PACK,
                "pack_footprint: 480/4800 attempts, best so far: 28 objects",
                inRepack = true,
            ),
        )
        // Без состояния repack та же строка — прогресс первой упаковки.
        assertEquals(
            JobPhase.PACKING to 16,
            ProgressRules.parse(JobKind.PACK, "pack_footprint: 480/4800 attempts, best so far: 28 objects"),
        )
    }

    @Test
    fun `пакер wrote с 3mf-путём — cleanup 95`() {
        assertEquals(
            JobPhase.CLEANUP to 95,
            ProgressRules.parse(JobKind.PACK, "packed 30 colored objects; wrote /tmp/j/packed.3mf"),
        )
    }

    @Test
    fun `нераспознанная строка — null`() {
        assertEquals(null, ProgressRules.parse(JobKind.PACK, "probe tower keepout: 10.00,10.00 → 30.00,30.00"))
    }

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
    fun `прогресс не откатывается назад`() {
        val tracker = ProgressTracker(JobKind.PACK)
        tracker.onLine("packed 30 objects; assembling 3MF...") // 75
        val event = tracker.onLine("pack_footprint: 240/2000 attempts, best so far: 12 objects") // ~17 — отбрасывается

        assertEquals(JobPhase.ASSEMBLING, event.phase)
        assertEquals(75, event.progressPercent)
        // Сообщение всё равно обновляется — это последняя значимая строка «как есть».
        assertEquals("pack_footprint: 240/2000 attempts, best so far: 12 objects", event.phaseMessage)
    }

    @Test
    fun `repacking — единственный допустимый откат`() {
        val tracker = ProgressTracker(JobKind.PACK)
        tracker.onLine("slicing final plate (30 objects)...") // 80
        val event = tracker.onLine("final tower outgrew probe keepout; repacking once")

        assertEquals(JobPhase.REPACKING, event.phase)
        assertEquals(40, event.progressPercent)
    }

    @Test
    fun `repack-проход — фаза repacking, растущий процент и префикс сообщения`() {
        val tracker = ProgressTracker(JobKind.PACK)
        tracker.onLine("packing (4800 attempts, seed 20270831)...")
        tracker.onLine("pack_footprint: 480/4800 attempts, best so far: 29 objects") // 16

        val start = tracker.onLine("final tower outgrew probe keepout and reaches placement 7; repacking once")
        assertEquals(JobPhase.REPACKING to 40, start.phase to start.progressPercent)

        // Прогресс второй упаковки: 40 + 30·X/N, процент растёт внутри repacking.
        val mid = tracker.onLine("pack_footprint: 480/4800 attempts, best so far: 28 objects")
        assertEquals(JobPhase.REPACKING, mid.phase)
        assertEquals(43, mid.progressPercent)
        assertEquals("Перепаковка: pack_footprint: 480/4800 attempts, best so far: 28 objects", mid.phaseMessage)

        // Слайс перепакованной пластины — снова слайсинг с продвижением вперёд.
        val slicing = tracker.onLine("slicing repacked plate (28 objects)...")
        assertEquals(JobPhase.SLICING to 80, slicing.phase to slicing.progressPercent)
    }

    @Test
    fun `проценты клампятся до 99, пока процесс жив`() {
        val tracker = ProgressTracker(JobKind.PACK)
        val event = tracker.onLine("pack_footprint: 2000/1000 attempts, best so far: 30 objects")

        assertEquals(JobPhase.PACKING, event.phase)
        assertEquals(99, event.progressPercent)
    }

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
    fun `пустые строки не меняют прогресс`() {
        val tracker = ProgressTracker(JobKind.PACK)
        tracker.onLine("probing prime tower position (slicing 1 object)...")
        val event = tracker.onLine("   ")

        assertEquals(JobPhase.PROBE, event.phase)
        assertEquals(5, event.progressPercent)
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
