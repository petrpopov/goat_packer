package com.goatpacker.web.config

import jakarta.annotation.PostConstruct
import org.springframework.boot.context.properties.ConfigurationProperties
import org.springframework.stereotype.Component
import java.nio.file.Path
import java.nio.file.Paths

/**
 * Пути из application.yml разворачиваются в абсолютные при старте: относительные пути
 * резолвятся от рабочего каталога процесса. При запуске `./gradlew bootRun` из webapp/backend
 * рабочим каталогом является сам webapp/backend, поэтому дефолт `../../target/release/...`
 * указывает на бинарники в корне репозитория goat_packer.
 */
@Component
@ConfigurationProperties(prefix = "app")
data class AppProperties(
    var packerBin: String = "../../target/release/colored_3mf_packer",
    var schedulerBin: String = "../../target/release/daily_scheduler",
    var profilesDir: String = "../../profiles",
    var jobsRoot: String = "${System.getProperty("java.io.tmpdir")}/goat-packer-web/jobs",
    var jobTtlHours: Long = 12,
    var allowedOrigins: List<String> = listOf("http://localhost:3007", "http://127.0.0.1:3007"),
) {
    lateinit var packerBinPath: Path
    lateinit var schedulerBinPath: Path
    lateinit var profilesDirPath: Path
    lateinit var jobsRootPath: Path

    @PostConstruct
    fun resolveAbsolutePaths() {
        packerBinPath = toAbsolute(packerBin)
        schedulerBinPath = toAbsolute(schedulerBin)
        profilesDirPath = toAbsolute(profilesDir)
        jobsRootPath = toAbsolute(jobsRoot)
    }

    private fun toAbsolute(raw: String): Path =
        Paths.get(raw).toAbsolutePath().normalize()
}
