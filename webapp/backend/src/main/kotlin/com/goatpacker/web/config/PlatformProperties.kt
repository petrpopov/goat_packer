package com.goatpacker.web.config

import com.goatpacker.web.entity.PrincipalKind
import org.springframework.boot.context.properties.ConfigurationProperties
import org.springframework.context.annotation.Profile
import org.springframework.stereotype.Component

/** Где исполняются задачи: в контейнерах Docker или процессами на хосте (только для разработки). */
enum class RunnerMode { DOCKER, LOCAL }

/** Где лежат объекты: S3-совместимое хранилище или каталог (тесты). */
enum class StorageMode { S3, FILE }

/** Настройки серверного режима (`platform.*`); окружение задаётся в application-server.yml. */
@Component
@Profile("server")
@ConfigurationProperties("platform")
class PlatformProperties {
    var secureCookies = true
    var adminLogin = ""
    var adminPassword = ""
    var publicUrl = "http://localhost:3007"
    var workerEnabled = false
    var workerId = "local-worker"
    var workerMemoryMib = 1536L
    var workerCpus = 2.0
    var maxParallel = 1
    var executionCpuBudget = 2.0
    var executionMemoryMib = 1536L
    var scratchLimitBytes = 2L * 1024 * 1024 * 1024
    var maxQueue = 20
    var maxUploadBytes = 50L * 1024 * 1024
    var outputReservationBytes = 50L * 1024 * 1024
    var storageLimitBytes = 10L * 1024 * 1024 * 1024
    var jobTimeoutSeconds = 1800L
    var s3Endpoint = "http://localhost:8333"
    var s3PublicEndpoint = "http://localhost:8333"
    var s3AccessKey = ""
    var s3SecretKey = ""
    var s3Bucket = "goat"
    var storageMode = StorageMode.S3
    var localStorageRoot = "./build/test-objects"
    var runnerMode = RunnerMode.DOCKER
    var runnerBambuImage = ""
    var runnerOrcaImage = ""
    var hostWorkRoot = "/srv/goat/scratch"
    var allowLocalRunner = false
    var production = false

    /**
     * Probe cache of the packer, kept by the worker per owner (see ProbeCacheService): off = every job probes anew.
     * Limits: one entry, entries replayed into one job (per model), bytes per owner, days since last use.
     */
    var probeCacheEnabled = true
    var probeCacheEntryMaxBytes = 16L * 1024 * 1024
    var probeCacheGroupMaxEntries = 16
    var probeCacheOwnerMaxBytes = 256L * 1024 * 1024
    var probeCacheTtlDays = 30
    var requireChallenge = false

    /**
     * Лимиты частоты (задачи в сутки, загрузки, сессии), активных задач на владельца, суточный
     * вычислительный бюджет и личная квота хранения. false — сайт закрыт снаружи (SSO Pangolin),
     * посторонних нет: свои работают без ограничений. Общий потолок хранилища, размер очереди и число
     * одновременных загрузок действуют всегда.
     */
    var admissionLimits = true
    var powDifficulty = 16
    var maxRetries = 1
    var guestComputeSeconds = 3600L
    var accountComputeSeconds = 10800L
    var globalComputeSeconds = 72000L
    var maxLogBytes = 10L * 1024 * 1024
    var uploadTimeoutSeconds = 300L

    /** Новых задач в сутки на владельца (проверка нарезкой не считается). */
    var guestJobsPerDay = 4
    var userJobsPerDay = 10

    /** Одновременно активных задач на владельца. */
    var guestActiveJobs = 2
    var userActiveJobs = 4

    /** Потолок числа гостей и пользователей сервиса. */
    var maxPrincipals = 10_000L

    /** Одновременно принимаемых загрузок на весь сервис (больше — «все места заняты»). */
    var maxActiveUploads = 2

    val localRunner: Boolean
        get() = runnerMode == RunnerMode.LOCAL

    fun jobsPerDay(kind: PrincipalKind): Int = if (kind == PrincipalKind.GUEST) guestJobsPerDay else userJobsPerDay

    fun activeJobsLimit(kind: PrincipalKind): Int = if (kind == PrincipalKind.GUEST) guestActiveJobs else userActiveJobs

    fun computeBudgetSeconds(kind: PrincipalKind): Long =
        if (kind == PrincipalKind.GUEST) guestComputeSeconds else accountComputeSeconds

    fun runnerImage(slicerKind: String): String =
        if (slicerKind == SlicerKind.BAMBU) runnerBambuImage else runnerOrcaImage
}
