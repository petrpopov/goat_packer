package com.goatpacker.web.security

import com.goatpacker.web.exception.ErrorCode
import jakarta.servlet.FilterChain
import jakarta.servlet.ReadListener
import jakarta.servlet.ServletInputStream
import jakarta.servlet.http.HttpServletRequest
import jakarta.servlet.http.HttpServletRequestWrapper
import jakarta.servlet.http.HttpServletResponse
import org.springframework.context.annotation.Profile
import org.springframework.core.Ordered
import org.springframework.core.annotation.Order
import org.springframework.http.HttpStatus
import org.springframework.stereotype.Component
import org.springframework.web.filter.OncePerRequestFilter
import java.io.ByteArrayInputStream

/**
 * Upload streams have their own reserved size; all control-plane bodies stay small,
 * including chunked requests without Content-Length. Bound memory before JSON binding.
 */
@Component
@Profile("server")
@Order(Ordered.HIGHEST_PRECEDENCE)
class RequestBodyLimitFilter : OncePerRequestFilter() {
    override fun doFilterInternal(req: HttpServletRequest, res: HttpServletResponse, chain: FilterChain) {
        if (req.method !in BODY_METHODS || UPLOAD_CONTENT_PATH.matches(req.requestURI)) {
            chain.doFilter(req, res)
            return
        }
        if (req.contentLengthLong > MAX_BODY_BYTES) return reject(res)
        val bytes = req.inputStream.readNBytes(MAX_BODY_BYTES + 1)
        if (bytes.size > MAX_BODY_BYTES) return reject(res)

        val stream = BufferedBody(bytes)
        chain.doFilter(
            object : HttpServletRequestWrapper(req) {
                override fun getInputStream() = stream

                override fun getReader() = stream.bufferedReader(Charsets.UTF_8)

                override fun getContentLength() = bytes.size

                override fun getContentLengthLong() = bytes.size.toLong()
            },
            res,
        )
    }

    private fun reject(res: HttpServletResponse) =
        writeError(res, HttpStatus.PAYLOAD_TOO_LARGE.value(), ErrorCode.BODY_TOO_LARGE, "Слишком большой запрос")

    /** Уже прочитанное тело запроса, отдаваемое повторно обработчику. */
    private class BufferedBody(bytes: ByteArray) : ServletInputStream() {
        private val body = ByteArrayInputStream(bytes)

        override fun read() = body.read()

        override fun read(b: ByteArray, off: Int, len: Int) = body.read(b, off, len)

        override fun isFinished() = body.available() == 0

        override fun isReady() = true

        override fun setReadListener(listener: ReadListener) =
            throw UnsupportedOperationException("Synchronous control-plane request")
    }

    private companion object {
        const val MAX_BODY_BYTES = 64 * 1024
        val BODY_METHODS = setOf("POST", "PUT", "PATCH")
        val UPLOAD_CONTENT_PATH = Regex("/api/v2/uploads/[a-f0-9-]{36}/content")
    }
}
