package com.goatpacker.web.server

import org.springframework.context.annotation.Profile
import org.springframework.data.domain.PageRequest
import org.springframework.jdbc.core.JdbcTemplate
import org.springframework.stereotype.Service
import org.springframework.transaction.annotation.Transactional
import org.springframework.web.bind.annotation.*
import java.time.Instant

@Service @Profile("server")
class Administration(private val owners:PrincipalRepository,private val sessions:SessionRepository,
    private val config:PlatformRepository,private val audit:AuditRepository,private val workers:WorkerRepository,
    private val quotas:Quotas,private val props:PlatformProperties,private val jdbc:JdbcTemplate) {
    @Transactional fun block(id:String,blocked:Boolean,reason:String) {
        config.locked();val target=owners.locked(id) ?: missing()
        if(target.id==actor().id || target.role=="ADMIN") bad("Нельзя блокировать себя или администратора")
        if(reason.trim().length !in 3..500) bad("Укажите причину (3–500 символов)")
        target.blocked=blocked;target.blockReason=if(blocked) reason.trim() else null
        if(blocked) {
            sessions.revoke(target.id)
            jdbc.update("update queue_jobs set cancel_requested=true where owner_id=? and status in ('QUEUED','RUNNING','FINALIZING','LOST','RETRY_WAIT')",id)
        }
        audit.save(AuditEntity(actorId=actor().id,targetId=id,action=if(blocked) "ACCESS_BLOCKED" else "ACCESS_UNBLOCKED",details=reason.trim()))
    }
    fun system():Map<String,Any> {
        val settings=config.findById(1).orElseThrow()
        val nodes=workers.findAll();val healthy=nodes.filter { it.lastSeenAt.isAfter(Instant.now().minusSeconds(30)) }
        val capacity=healthy.sumOf { it.capacity }
        return mapOf("configuredParallelism" to settings.maxParallel,"effectiveParallelism" to if(settings.paused) 0 else minOf(settings.maxParallel,capacity),
            "lostJobs" to jdbc.queryForObject("select count(*) from queue_jobs where status='LOST'",Long::class.java)!!,
            "reportedActiveJobs" to healthy.sumOf { it.activeCount },
            "paused" to settings.paused,"activeJobs" to jdbc.queryForObject("select count(*) from queue_jobs where status in ('RUNNING','FINALIZING')",Long::class.java)!!,
            "queuedJobs" to jdbc.queryForObject("select count(*) from queue_jobs where status in ('QUEUED','RETRY_WAIT')",Long::class.java)!!,
            "workers" to nodes,"storageUsedBytes" to quotas.totalBytes(),"storageLimitBytes" to props.storageLimitBytes,
            "apiAvailableProcessors" to Runtime.getRuntime().availableProcessors(),"apiHeapMaxBytes" to Runtime.getRuntime().maxMemory())
    }
    @Transactional fun configure(maxParallel:Int,paused:Boolean,reason:String) {
        if(maxParallel !in 0..64 || reason.trim().length !in 3..500) bad("Параллелизм 0–64 и причина обязательны")
        val setting=config.locked();val old=setting.maxParallel
        setting.maxParallel=maxParallel;setting.paused=paused
        audit.save(AuditEntity(actorId=actor().id,action="SYSTEM_LIMIT_UPDATED",details="parallelism=$old->$maxParallel paused=$paused reason=${reason.trim()}"))
    }
}
@RestController @RequestMapping("/api/v2/admin") @Profile("server")
class AdminController(private val owners:PrincipalRepository,private val audit:AuditRepository,
    private val ids:Identities,private val admin:Administration,private val quotas:Quotas) {
    data class BlockRequest(val blocked:Boolean=true,val reason:String="")
    data class Reason(val reason:String="")
    data class Settings(val maxParallel:Int=1,val paused:Boolean=false,val reason:String="")
    @GetMapping("/users") fun users(@RequestParam(defaultValue="") kind:String,@RequestParam(defaultValue="") query:String,
        @RequestParam(defaultValue="0") page:Int):Map<String,Any> {
        if(kind !in listOf("","GUEST","USER") || query.length>100 || page !in 0..10000) bad("Некорректный фильтр")
        val result=owners.search(kind,query.lowercase(),PageRequest.of(page,30,org.springframework.data.domain.Sort.by("createdAt").descending()))
        return mapOf("items" to result.content.map { ids.view(it)+mapOf("createdAt" to it.createdAt,"lastSeenAt" to it.lastSeenAt,"blockReason" to it.blockReason,"resetRequired" to it.resetRequired,"storageUsedBytes" to quotas.ownerBytes(it.id)) },"total" to result.totalElements,"page" to page)
    }
    @PostMapping("/users/{id}/block") fun block(@PathVariable id:String,@RequestBody request:BlockRequest) {admin.block(id,request.blocked,request.reason)}
    @PostMapping("/users/{id}/password-reset") fun reset(@PathVariable id:String,@RequestBody request:Reason)=mapOf("token" to ids.issueReset(id,request.reason),"expiresInSeconds" to 900)
    @GetMapping("/system") fun system()=admin.system()
    @PostMapping("/system") fun system(@RequestBody request:Settings) { admin.configure(request.maxParallel,request.paused,request.reason) }
    @GetMapping("/audit") fun audit(@RequestParam(defaultValue="0") page:Int):Any {
        if(page !in 0..10000) bad("Некорректная страница")
        return audit.findAllByOrderByCreatedAtDesc(PageRequest.of(page,30)).content
    }
}
