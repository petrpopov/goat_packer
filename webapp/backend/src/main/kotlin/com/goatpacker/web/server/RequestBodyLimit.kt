package com.goatpacker.web.server

import jakarta.servlet.*
import jakarta.servlet.http.*
import org.springframework.context.annotation.Profile
import org.springframework.core.Ordered
import org.springframework.core.annotation.Order
import org.springframework.stereotype.Component
import org.springframework.web.filter.OncePerRequestFilter
import java.io.ByteArrayInputStream

/** Upload streams have their own reserved size; all control-plane bodies stay small,
 * including chunked requests without Content-Length. Bound memory before JSON binding. */
@Component @Profile("server") @Order(Ordered.HIGHEST_PRECEDENCE)
class RequestBodyLimit:OncePerRequestFilter() {
    override fun doFilterInternal(req:HttpServletRequest,res:HttpServletResponse,chain:FilterChain) {
        if(req.method !in setOf("POST","PUT","PATCH") || Regex("/api/v2/uploads/[a-f0-9-]{36}/content").matches(req.requestURI)) {
            chain.doFilter(req,res);return
        }
        val limit=64*1024
        fun reject() {res.status=413;res.contentType="application/json";res.writer.write("""{"code":"BODY_TOO_LARGE","message":"Слишком большой запрос"}""")}
        if(req.contentLengthLong>limit) {reject();return}
        val bytes=req.inputStream.readNBytes(limit+1)
        if(bytes.size>limit) {reject();return}
        val body=ByteArrayInputStream(bytes)
        val stream=object:ServletInputStream() {
            override fun read()=body.read()
            override fun read(b:ByteArray,off:Int,len:Int)=body.read(b,off,len)
            override fun isFinished()=body.available()==0
            override fun isReady()=true
            override fun setReadListener(listener:ReadListener) {throw UnsupportedOperationException("Synchronous control-plane request")}
        }
        chain.doFilter(object:HttpServletRequestWrapper(req) {
            override fun getInputStream()=stream
            override fun getReader()=stream.bufferedReader(Charsets.UTF_8)
            override fun getContentLength()=bytes.size
            override fun getContentLengthLong()=bytes.size.toLong()
        },res)
    }
}
