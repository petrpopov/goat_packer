package com.goatpacker.web.service

import com.goatpacker.web.job.JobKind
import com.goatpacker.web.job.JobPhase
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertFalse
import org.junit.jupiter.api.Assertions.assertNull
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test

/** Прогресс пакера по stage-событиям: полосы, интерполяция по времени, монотонность, без JSON. */
class PackStageProgressTest {

    /** Ручные часы: тесты двигают время явно. */
    private class FakeClock(var now: Long = 1_000_000L) {
        fun advance(seconds: Double) {
            now += (seconds * 1000).toLong()
        }
    }

    private fun tracker(clock: FakeClock) = ProgressTracker(JobKind.PACK) { clock.now }

    private fun stage(id: String, expected: Double? = null, objects: Int? = null): String =
        """{"event":"stage","stage":"$id","message":"english text","expected_s":${expected ?: "null"},"objects":${objects ?: "null"}}"""

    private fun heartbeat(ms: Long) = """{"event":"slicer_progress","elapsed_ms":$ms}"""

    @Test
    fun `stage-событие открывает этап — фаза, начало полосы, русское сообщение`() {
        val clock = FakeClock()
        val t = tracker(clock)
        val probe = t.onLine(stage("probe_slice", 22.0, 1))

        assertEquals(JobPhase.PROBE, probe.phase)
        assertEquals(PackStage.PROBE_SLICE.bandStart, probe.progressPercent)
        assertEquals("Пробная нарезка 1 объекта: ищу место башни очистки", probe.phaseMessage)
        assertEquals("probe_slice", probe.stage?.id)
        assertEquals(22.0, probe.stage?.expectedSeconds)
        assertEquals(clock.now, probe.stage?.startedAtMs)

        val final = t.onLine(stage("final_slice", 317.0, 37))
        assertEquals(JobPhase.SLICING, final.phase)
        assertEquals(PackStage.FINAL_SLICE.bandStart, final.progressPercent)
        assertEquals("Нарезка итоговой пластины (37 объектов)", final.phaseMessage)
        assertEquals(listOf("probe_slice", "final_slice"), final.stage?.visited)
    }

    @Test
    fun `heartbeat двигает процент по времени и не доходит до конца полосы`() {
        val clock = FakeClock()
        val t = tracker(clock)
        t.onLine(stage("final_slice", 100.0, 20))

        clock.advance(50.0)
        val half = t.onLine(heartbeat(50_000))
        // 33 + 45 * 0.5 = 55.5
        assertEquals(55, half.progressPercent)

        clock.advance(50.0)
        val onTime = t.onLine(heartbeat(100_000))
        assertTrue(onTime.progressPercent in 56 until PackStage.FINAL_SLICE.bandEnd, "$onTime")

        // Нарезка затянулась в десять раз — шкала ползёт, но конец полосы не достигается.
        clock.advance(900.0)
        val late = t.onLine(heartbeat(1_000_000))
        assertTrue(late.progressPercent < PackStage.FINAL_SLICE.bandEnd, "$late")
        assertTrue(late.progressPercent >= onTime.progressPercent)
        // Сообщение остаётся русским описанием этапа, а не JSON heartbeat'а.
        assertEquals("Нарезка итоговой пластины (20 объектов)", late.phaseMessage)
    }

    @Test
    fun `без expected_s нарезка интерполируется по оценке из числа объектов`() {
        val clock = FakeClock()
        val t = tracker(clock)
        t.onLine(stage("final_slice", null, 37))
        // Оценка 22 * (1 + 0.36 * 36) ≈ 307 с; через 30 с — заметно больше начала полосы, но не конец.
        clock.advance(30.0)
        val event = t.onLine(heartbeat(30_000))
        assertTrue(event.progressPercent in 36..40, "$event")
        assertNull(event.stage?.expectedSeconds, "оставшееся время без оценки пакера не показывается")
    }

    @Test
    fun `slicer_output и прочие JSON никогда не становятся сообщением`() {
        val clock = FakeClock()
        val t = tracker(clock)
        t.onLine(stage("probe_slice", 22.0, 1))
        for (line in listOf(
            """{"event":"slicer_started","pid":42}""",
            """{"event":"slicer_output","stream":"stdout","message":"[info] slicing layer 3"}""",
            """{"event":"slicer_finished","exit_code":0,"elapsed_ms":21000}""",
            heartbeat(2000),
            """{"weird":"json"}""",
        )) {
            val event = t.onLine(line)
            assertFalse(event.phaseMessage.contains("{"), line)
            assertFalse(event.stage?.detail.orEmpty().contains("{"), line)
        }
    }

    @Test
    fun `английская строка — вторичная подробность, сообщение остаётся русским`() {
        val clock = FakeClock()
        val t = tracker(clock)
        t.onLine(stage("packing"))
        val event = t.onLine("pack_footprint: 32/64 attempts, best so far: 37 objects")

        assertEquals("Раскладка объектов: лучший вариант — 37 объектов", event.phaseMessage)
        assertEquals("pack_footprint: 32/64 attempts, best so far: 37 objects", event.stage?.detail)
        // 15 + 15 * 0.5 * 0.95 ≈ 22
        assertEquals(22, event.progressPercent)
    }

    @Test
    fun `полный путь с перепаковкой и окном — процент не убывает`() {
        val clock = FakeClock()
        val t = tracker(clock)
        val lines = listOf(
            stage("probe_slice", 22.0, 1), heartbeat(2000), "probing prime tower position (slicing 1 object)...",
            stage("packing"), "pack_footprint: 64/64 attempts, best so far: 37 objects",
            stage("assembling", null, 37), "packed 37 objects; assembling 3MF...",
            stage("final_slice", 317.0, 37), heartbeat(2000), heartbeat(4000),
            stage("gcode_check"),
            stage("repacking"), "pack_footprint: 10/64 attempts, best so far: 35 objects",
            stage("assembling", null, 35),
            stage("final_slice", 300.0, 35), heartbeat(2000),
            stage("window_slice", 120.0, 20), heartbeat(2000),
            stage("window_slice", 100.0, 16), heartbeat(2000),
            stage("window_slice", 90.0, 15), heartbeat(2000),
            stage("writing"), "report: /work/packed.verification.json",
        )
        var previous = 0
        for (line in lines) {
            clock.advance(5.0)
            val event = t.onLine(line)
            assertTrue(event.progressPercent >= previous, "откат на «$line»: $previous → ${event.progressPercent}")
            assertTrue(event.progressPercent <= 99)
            assertFalse(event.phaseMessage.contains("{"))
            previous = event.progressPercent
        }
        assertEquals("writing", t.currentStage?.id)
        assertEquals(
            listOf("probe_slice", "packing", "assembling", "final_slice", "gcode_check", "repacking", "window_slice", "writing"),
            t.currentStage?.visited,
        )
    }

    @Test
    fun `перепаковка — своя полоса после итоговой нарезки, повторная нарезка продолжает вперёд`() {
        val clock = FakeClock()
        val t = tracker(clock)
        t.onLine(stage("final_slice", 100.0, 37))
        clock.advance(60.0)
        val sliced = t.onLine(heartbeat(60_000))

        val repack = t.onLine(stage("repacking"))
        assertEquals(JobPhase.REPACKING, repack.phase)
        assertEquals(PackStage.REPACKING.bandStart, repack.progressPercent)
        assertTrue(repack.progressPercent >= sliced.progressPercent)

        val reslice = t.onLine(stage("final_slice", 100.0, 35))
        assertTrue(reslice.progressPercent >= repack.progressPercent)
        assertEquals("Нарезка перепакованной пластины (35 объектов)", reslice.phaseMessage)
        assertEquals(2, reslice.stage?.attempt)
    }

    @Test
    fun `причина перепаковки из события попадает в сообщение`() {
        val reasons = mapOf(
            "support_contact" to "Перепаковка: поддержки задели соседей, расширяю зону поддержек",
            "support_strict" to "Перепаковка по строгому правилу: поддержки не заходят под соседей",
            "tower_corner_failed" to "Перепаковка: башня в углу не прошла нарезку, возвращаю её на место",
            "tower_grown" to "Перепаковка: башня очистки выросла",
        )
        for ((reason, text) in reasons) {
            val t = tracker(FakeClock())
            val event = t.onLine(
                """{"event":"stage","stage":"repacking","message":"re-packing","expected_s":null,"objects":null,"reason":"$reason"}""",
            )
            assertEquals(text, event.phaseMessage)
        }
        // Без reason (пакер до этого изменения) — как раньше.
        assertEquals("Перепаковка: башня очистки выросла", tracker(FakeClock()).onLine(stage("repacking")).phaseMessage)
    }

    @Test
    fun `повторные нарезки окна — номер попытки в сообщении`() {
        val clock = FakeClock()
        val t = tracker(clock)
        t.onLine(stage("window_slice", 120.0, 21))
        val second = t.onLine(stage("window_slice", 100.0, 18))

        assertEquals(2, second.stage?.attempt)
        assertEquals("Подбор под окно времени: нарезка 18 объектов, попытка 2", second.phaseMessage)
    }

    @Test
    fun `текстовая строка до события того же этапа — событие дополняет этап, а не перезапускает`() {
        val clock = FakeClock()
        val t = tracker(clock)
        val started = clock.now
        t.onLine("probing prime tower position (slicing 1 object)...")
        clock.advance(3.0)
        val event = t.onLine(stage("probe_slice", 22.0, 1))

        assertEquals(started, event.stage?.startedAtMs)
        assertEquals(22.0, event.stage?.expectedSeconds)
        assertEquals(1, event.stage?.attempt)
    }

    @Test
    fun `после stage-событий текстовые строки этапы не переключают`() {
        val clock = FakeClock()
        val t = tracker(clock)
        t.onLine(stage("final_slice", 300.0, 30))
        val event = t.onLine("packed 30 objects; assembling 3MF...")

        assertEquals("final_slice", event.stage?.id)
        assertEquals("packed 30 objects; assembling 3MF...", event.stage?.detail)
    }

    @Test
    fun `без stage-событий этапы распознаются по английским строкам (локальный режим)`() {
        val clock = FakeClock()
        val t = tracker(clock)
        val expected = listOf(
            "probing prime tower position (slicing 1 object)..." to "probe_slice",
            "probe tower keepout: 10.00,10.00 → 30.00,30.00" to "probe_slice",
            "packing (16000 attempts, seed 20270831)..." to "packing",
            "pack_footprint: 64/64 attempts, best so far: 30 objects" to "packing",
            "packed 30 objects; assembling 3MF..." to "assembling",
            "slicing final plate (30 objects)..." to "final_slice",
            "final tower outgrew probe keepout and reaches placement 7; repacking once" to "repacking",
            "re-packing (16000 attempts, seed 20270831)..." to "repacking",
            "packed 28 objects; assembling 3MF..." to "assembling",
            "slicing repacked plate (28 objects)..." to "final_slice",
            "slicing time-window plate (12 objects)..." to "window_slice",
            "slicing time-window plate (10 objects)..." to "window_slice",
            "packed 10 colored objects; wrote packed.3mf" to "writing",
        )
        var previous = 0
        for ((line, id) in expected) {
            clock.advance(2.0)
            val event = t.onLine(line)
            assertEquals(id, event.stage?.id, line)
            assertTrue(event.progressPercent >= previous, line)
            previous = event.progressPercent
        }
        assertEquals(1, t.currentStage?.attempt)
        assertEquals("Запись результата", t.message)
        assertNull(t.currentStage?.expectedSeconds)
    }

    @Test
    fun `до первого этапа сообщение пустое, пустые строки ничего не меняют`() {
        val t = tracker(FakeClock())
        val first = t.onLine("loading source…")
        assertEquals("", first.phaseMessage)
        assertNull(first.stage)
        assertEquals(0, first.progressPercent)
        assertEquals(0, t.onLine("   ").progressPercent)
    }

    @Test
    fun `процент клампится до 99, пока процесс жив`() {
        val clock = FakeClock()
        val t = tracker(clock)
        t.onLine(stage("writing"))
        repeat(10) {
            t.onLine(stage("window_slice", 1.0, 1))
            clock.advance(100.0)
            t.onLine(heartbeat(100_000))
        }
        assertTrue(t.percent <= 99)
    }

    @Test
    fun `доля по времени — линейна до 80 процентов и асимптотична после`() {
        assertEquals(0.5, ProgressTracker.timeFraction(50.0, 100.0), 1e-9)
        assertEquals(0.8, ProgressTracker.timeFraction(80.0, 100.0), 1e-9)
        assertTrue(ProgressTracker.timeFraction(100.0, 100.0) in 0.8..0.95)
        assertTrue(ProgressTracker.timeFraction(10_000.0, 100.0) <= 0.95)
    }

    // ── Планировщик и калибровка: JSON слайсера не ломает их прогресс ───────────────────

    @Test
    fun `планировщик игнорирует JSON-события слайсера`() {
        val t = ProgressTracker(JobKind.SCHEDULE)
        t.onLine("[Bambu Lab A2L] slicing sample 1/5: batch 8/30")
        val event = t.onLine("""{"event":"slicer_output","stream":"stdout","message":"objects, jobs ["}""")

        assertEquals(JobPhase.BENCHMARK to 23, event.phase to event.progressPercent)
        assertEquals("[Bambu Lab A2L] slicing sample 1/5: batch 8/30", event.phaseMessage)
        assertNull(event.stage)
    }

    @Test
    fun `калибровка не меняется от JSON-событий слайсера`() {
        val t = ProgressTracker(JobKind.CALIBRATION)
        val event = t.onLine("""{"event":"slicer_started","pid":7}""")
        assertEquals(ProgressTracker.CALIBRATION_MESSAGE, event.phaseMessage)
        assertEquals(10, event.progressPercent)
        val done = t.onLine("  \"changes_sliced\": 10,")
        assertEquals(JobPhase.WRITING to 95, done.phase to done.progressPercent)
    }
}
