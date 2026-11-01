package com.goatpacker.web.model

/** Тип джоба: упаковка 3MF, построение расписания или калибровка времени смены AMS. */
enum class JobKind {
    PACK,
    SCHEDULE,
    CALIBRATION;

    /** Значение для фронтенда: `pack|schedule|calibration`. */
    fun jsonValue(): String = name.lowercase()
}

/** Жизненный цикл джоба в памяти (локальный режим и состояние исполнения на воркере). */
enum class JobStatus {
    QUEUED,
    RUNNING,
    SUCCEEDED,
    FAILED
}

/**
 * Фаза выполнения. Значения — ровно те, что уходят фронту:
 * upload|probe|packing|assembling|slicing|repacking|benchmark|optimizing|writing|cleanup.
 */
enum class JobPhase(val json: String) {
    UPLOAD("upload"),
    PROBE("probe"),
    PACKING("packing"),
    ASSEMBLING("assembling"),
    SLICING("slicing"),
    REPACKING("repacking"),
    BENCHMARK("benchmark"),
    OPTIMIZING("optimizing"),
    WRITING("writing"),
    CLEANUP("cleanup");

    fun jsonValue(): String = json
}
