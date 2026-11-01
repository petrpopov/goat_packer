package com.goatpacker.web.config

import com.goatpacker.web.repository.FileObjectStore
import com.goatpacker.web.repository.ObjectStore
import com.goatpacker.web.repository.S3ObjectStore
import com.goatpacker.web.service.server.AccountService
import org.springframework.boot.ApplicationRunner
import org.springframework.context.annotation.Bean
import org.springframework.context.annotation.Configuration
import org.springframework.context.annotation.Profile
import org.springframework.security.crypto.bcrypt.BCryptPasswordEncoder
import org.springframework.security.crypto.password.PasswordEncoder
import java.nio.file.Path

/** Общие бины серверного режима: хеширование паролей, хранилище объектов, стартовый администратор. */
@Configuration
@Profile("server")
class ServerBeansConfig {
    @Bean
    fun passwordEncoder(): PasswordEncoder = BCryptPasswordEncoder(BCRYPT_STRENGTH)

    @Bean
    fun objectStore(props: PlatformProperties): ObjectStore = when (props.storageMode) {
        StorageMode.FILE -> FileObjectStore(Path.of(props.localStorageRoot).toAbsolutePath())
        StorageMode.S3 -> S3ObjectStore(props)
    }

    @Bean
    fun bootstrapAdministrator(accounts: AccountService) = ApplicationRunner { accounts.bootstrapAdministrator() }

    private companion object {
        /** Стоимость bcrypt: баланс между задержкой входа и стойкостью к перебору. Менять нельзя — хеши уже в БД. */
        const val BCRYPT_STRENGTH = 12
    }
}
