package com.goatpacker.web.service.server

import com.goatpacker.web.dto.HealthDto
import com.goatpacker.web.repository.OperationalStateRepository
import org.springframework.context.annotation.Profile
import org.springframework.stereotype.Service

@Service
@Profile("server")
class HealthService(private val operationalState: OperationalStateRepository) {
    /** Живость API вместе с базой данных: без неё очередь не работает. */
    fun status(): HealthDto {
        operationalState.ping()
        return HealthDto("ok", "postgres")
    }
}
