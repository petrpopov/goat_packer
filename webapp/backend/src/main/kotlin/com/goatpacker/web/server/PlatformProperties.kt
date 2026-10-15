package com.goatpacker.web.server

import org.springframework.boot.context.properties.ConfigurationProperties
import org.springframework.context.annotation.Profile
import org.springframework.stereotype.Component

@Component @Profile("server") @ConfigurationProperties("platform")
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
    var storageMode = "s3"
    var localStorageRoot = "./build/test-objects"
    var runnerMode = "docker"
    var runnerBambuImage = ""
    var runnerOrcaImage = ""
    var hostWorkRoot = "/srv/goat/scratch"
    var allowLocalRunner = false
    var production = false
    var requireChallenge = false
    var powDifficulty = 16
    var maxRetries = 1
    var guestComputeSeconds = 3600L
    var accountComputeSeconds = 10800L
    var globalComputeSeconds = 72000L
    var maxLogBytes = 10L * 1024 * 1024
    var uploadTimeoutSeconds = 300L
}
