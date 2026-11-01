package com.goatpacker.web.entity

import jakarta.persistence.Entity
import jakarta.persistence.Id
import jakarta.persistence.Table
import jakarta.persistence.Version
import java.time.Instant

/** Единственная строка (id = 1) с настройками, которые админ меняет без перезапуска. */
@Entity
@Table(name = "platform_config")
class PlatformEntity(
    @Id var id: Int = PLATFORM_CONFIG_ID,
    var maxParallel: Int = 1,
    var paused: Boolean = false,
    @Version var version: Long = 0,
) {
    companion object {
        const val PLATFORM_CONFIG_ID = 1

        /** Верхняя граница числа одновременно работающих задач, которую можно задать. */
        const val MAX_PARALLELISM = 64
    }
}

/** Исполнитель задач: его heartbeat и ёмкость определяют число одновременных запусков. */
@Entity
@Table(name = "worker_nodes")
class WorkerEntity(
    @Id var id: String = "",
    var bootId: String = "",
    var cpuLimit: Double = 0.0,
    var memoryLimitBytes: Long = 0,
    var capacity: Int = 0,
    var activeCount: Int = 0,
    var lastSeenAt: Instant = Instant.now(),
    var mode: String = "docker",
    var hostId: String = "",
)
