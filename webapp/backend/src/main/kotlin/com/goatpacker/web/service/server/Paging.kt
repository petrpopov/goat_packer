package com.goatpacker.web.service.server

import com.goatpacker.web.exception.ErrorMessages
import com.goatpacker.web.exception.badRequest

/** Страничный просмотр списков: номер страницы ограничен, чтобы offset не рос без предела. */
object Paging {
    val PAGE_RANGE = 0..10_000

    fun requirePage(page: Int) {
        if (page !in PAGE_RANGE) badRequest(ErrorMessages.INVALID_PAGE)
    }
}
