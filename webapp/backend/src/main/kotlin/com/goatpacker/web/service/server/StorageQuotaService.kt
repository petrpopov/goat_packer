package com.goatpacker.web.service.server

import com.goatpacker.web.config.PlatformProperties
import com.goatpacker.web.entity.PrincipalEntity
import com.goatpacker.web.exception.accessBlocked
import com.goatpacker.web.exception.quotaExceeded
import com.goatpacker.web.repository.StorageUsageRepository
import org.springframework.context.annotation.Profile
import org.springframework.stereotype.Service

/** Резервирование места в хранилище под загрузки и результаты: личная квота и общий потолок диска. */
@Service
@Profile("server")
class StorageQuotaService(
    private val usage: StorageUsageRepository,
    private val props: PlatformProperties,
) {
    /** Бросает 429, если [bytes] не помещаются в квоту [owner] или в общий лимит хранилища. */
    fun reserve(owner: PrincipalEntity, bytes: Long) {
        if (owner.blocked) accessBlocked()
        // Без лимитов (ADMISSION_LIMITS=false) личной квоты нет; общий потолок хранилища — всегда (диск).
        if (bytes < 0 || (props.admissionLimits && bytes > owner.storageQuota - usage.ownerBytes(owner.id))) {
            quotaExceeded("Квота хранения исчерпана")
        }
        if (bytes > props.storageLimitBytes - usage.totalBytes()) quotaExceeded("Хранилище временно заполнено")
    }
}
