package com.goatpacker.web.server

import com.fasterxml.jackson.databind.ObjectMapper
import com.goatpacker.web.service.ProfileService
import org.springframework.context.annotation.Profile
import org.springframework.data.domain.PageRequest
import org.springframework.jdbc.core.JdbcTemplate
import org.springframework.stereotype.Service
import org.springframework.transaction.annotation.Transactional
import org.springframework.web.bind.annotation.*
import jakarta.servlet.http.HttpServletRequest
import java.nio.file.Files
import java.time.Instant

val activeStates=setOf("QUEUED","RUNNING","FINALIZING","LOST","RETRY_WAIT")
data class JobRequest(val kind:String="PACK",val fileId:String?=null,val packJobId:String?=null,val profileId:String="",
    val clearance:Double=0.7,val resolution:Double=0.25,val attempts:Long=4800,val edgeMargin:Double?=null,
    val layout:String="search",val nozzleClean:String="",val dayStart:String="10:00",val nightStart:String="23:00",
    val horizonEnd:String="34:00",val turnaroundMin:Int=10,val prepMin:Int=10,val benchMode:String="adaptive")
data class CreatedJob(val jobId:String,val readToken:String?=null)

@Service @Profile("server")
class DurableQueue(private val jobs:QueueJobRepository,private val owners:PrincipalRepository,private val uploads:UploadRepository,
    private val blobs:BlobRepository,private val artifacts:ArtifactRepository,private val platform:PlatformRepository,
    private val profiles:ProfileService,private val quotas:Quotas,private val mapper:ObjectMapper,
    private val props:PlatformProperties,private val jdbc:JdbcTemplate,private val limits:AdmissionLimits) {
    @Transactional fun create(ownerId:String,key:String,request:JobRequest):CreatedJob {
        if(!Regex("[a-zA-Z0-9_-]{8,100}").matches(key)) bad("Нужен Idempotency-Key длиной 8–100")
        validate(request)
        platform.locked();val owner=owners.locked(ownerId) ?: missing()
        if(jdbc.queryForObject("select maintenance from operational_state where id=1",Boolean::class.java)==true) throw ServerError(503,"MAINTENANCE","Сервис на обслуживании")
        val hash=Secrets.hash(mapper.writeValueAsString(request))
        jobs.findByOwnerIdAndRequestKey(ownerId,key)?.let {
            if(it.requestHash!=hash) throw ServerError(409,"CONFLICT","Этот ключ запроса использован с другими параметрами")
            return CreatedJob(it.id)
        }
        val (input,name)=if(request.fileId!=null) {
            val file=uploads.findById(request.fileId).orElse(null)?.takeIf { it.ownerId==ownerId && it.state=="READY" } ?: missing()
            if(file.expiresAt?.isBefore(Instant.now())==true) throw ServerError(410,"EXPIRED","Файл истёк")
            if(file.profileId!=request.profileId) bad("Принтер файла не соответствует выбранному профилю")
            file.blobId!! to file.originalName
        } else {
            val previous=owned(request.packJobId!!,ownerId)
            if(previous.status!="SUCCEEDED" || previous.kind!="PACK" || previous.profileId!=request.profileId) bad("Нужна успешная упаковка того же принтера")
            val packed=artifacts.findAllByJobId(previous.id).find { it.name=="packed.3mf" } ?: missing()
            // Output promoted to an immutable input blob reference; no dependency on parent's local files.
            val existing=jdbc.query("select id from blobs where object_key=?",{rs,_->rs.getString(1)},packed.objectKey).firstOrNull()
            val blob=existing ?: blobs.save(BlobEntity(sha256="artifact:${packed.id}",sizeBytes=packed.sizeBytes,objectKey=packed.objectKey)).id
            blob to previous.originalName
        }
        limits.hit("jobs:$ownerId",if(owner.kind=="GUEST") 4 else 10,86400)
        val current=jdbc.queryForObject("select count(*) from queue_jobs where status in ('QUEUED','RUNNING','FINALIZING','LOST','RETRY_WAIT')",Long::class.java)!!
        if(current>=props.maxQueue) limited("Очередь заполнена — попробуйте позже")
        val ownedCount=jdbc.queryForObject("select count(*) from queue_jobs where owner_id=? and status in ('QUEUED','RUNNING','FINALIZING','LOST','RETRY_WAIT')",Long::class.java,ownerId)!!
        if(ownedCount>=if(owner.kind=="GUEST") 2 else 4) limited("Достигнут лимит ваших активных задач")
        quotas.reserve(owner,props.outputReservationBytes)
        val budgetDay=java.time.LocalDate.now(java.time.ZoneOffset.UTC)
        reserveCompute(owner,budgetDay,props.jobTimeoutSeconds)
        val profile=profiles.require(request.profileId)
        if(request.nozzleClean.isNotBlank() && !profile.dto.supportsNozzleClean) bad("Принтер не поддерживает чистку сопла")
        if(owner.kind=="GUEST" && request.benchMode=="full") bad("Полный перебор партий доступен только аккаунту")
        val token=Secrets.token()
        val j=jobs.save(QueueJobEntity(ownerId=ownerId,kind=request.kind,inputBlobId=input,originalName=name,
            profileId=request.profileId,profileJson=Files.readString(profile.file),paramsJson=mapper.writeValueAsString(request),
            requestKey=key,requestHash=hash,readTokenHash=Secrets.hash(token),reservedBytes=props.outputReservationBytes,
            computeReservedSeconds=props.jobTimeoutSeconds,budgetDay=budgetDay))
        return CreatedJob(j.id,token)
    }
    fun owned(id:String,owner:String):QueueJobEntity=jobs.findById(id).orElse(null)?.takeIf { it.ownerId==owner }?.also(::notExpired) ?: missing()
    fun readable(id:String,owner:String,token:String?):QueueJobEntity {
        val j=jobs.findById(id).orElse(null) ?: missing()
        val grant=jdbc.queryForObject("select count(*) from read_grants where owner_id=? and job_id=? and expires_at>now()",Long::class.java,owner,id)!!>0
        if(j.ownerId!=owner && !grant && (token==null || token.length>100 || j.readTokenHash!=Secrets.hash(token))) missing()
        if(owners.findById(j.ownerId).orElse(null)?.blocked!=false) missing()
        notExpired(j);return j
    }
    private fun notExpired(j:QueueJobEntity) { if(j.expiresAt?.isBefore(Instant.now())==true) throw ServerError(410,"EXPIRED","Срок хранения задачи истёк") }
    @Transactional fun grant(id:String,owner:String,token:String) {
        val j=readable(id,owner,token)
        jdbc.update("insert into read_grants(owner_id,job_id,expires_at) values (?,?,?) on conflict(owner_id,job_id) do update set expires_at=excluded.expires_at",owner,id,java.sql.Timestamp.from(j.expiresAt ?: Instant.now().plusSeconds(86400)))
    }
    @Transactional fun share(id:String,owner:String):String { platform.locked();val j=owned(id,owner);jdbc.update("delete from read_grants where job_id=?",id);val token=Secrets.token();j.readTokenHash=Secrets.hash(token);return token }
    @Transactional fun cancel(id:String,owner:String) {
        platform.locked();val j=owned(id,owner)
        if(j.status in activeStates) {j.cancelRequested=true;if(j.status in setOf("QUEUED","RETRY_WAIT")) terminal(j,"CANCELLED",null)}
    }
    @Transactional fun delete(id:String,owner:String) {
        platform.locked();val j=owned(id,owner)
        if(j.status in activeStates) {j.cancelRequested=true;if(j.status in setOf("QUEUED","RETRY_WAIT")) terminal(j,"CANCELLED",null);return}
        jdbc.update("""insert into object_gc(object_key,size_bytes,not_before)
            select a.object_key,a.size_bytes,now() from artifacts a where a.job_id=?
            and not exists(select 1 from blobs b where b.object_key=a.object_key) on conflict do nothing""",id)
        jdbc.update("delete from queue_jobs where id=?",id)
    }
    @Transactional fun claim(worker:String,bootId:String?=null):QueueJobEntity? {
        val cfg=platform.locked();if(cfg.paused || cfg.maxParallel==0 || jdbc.queryForObject("select maintenance from operational_state where id=1",Boolean::class.java)==true) return null
        if(bootId!=null && jdbc.queryForObject("select count(*) from worker_nodes where id=? and boot_id=?",Long::class.java,worker,bootId)==0L) return null
        val capacity=jdbc.queryForObject("select coalesce(sum(capacity),0) from worker_nodes where last_seen_at>now()-interval '30 seconds'",Int::class.java)!!
        val ownCapacity=jdbc.query("select capacity from worker_nodes where id=? and last_seen_at>now()-interval '30 seconds'",{r,_->r.getInt(1)},worker).firstOrNull() ?: return null
        val ownActive=jdbc.queryForObject("select count(*) from queue_jobs where worker_id=? and status in ('RUNNING','FINALIZING','LOST')",Long::class.java,worker)!!
        if(ownActive>=ownCapacity) return null
        val active=jdbc.queryForObject("select count(*) from queue_jobs where status in ('RUNNING','FINALIZING','LOST')",Long::class.java)!!
        if(active>=minOf(cfg.maxParallel,capacity)) return null
        val id=jdbc.query("""select j.id from queue_jobs j join principals p on p.id=j.owner_id
            where j.status in ('QUEUED','RETRY_WAIT') and j.available_at<=now() and not j.cancel_requested and not p.blocked
            and not exists(select 1 from queue_jobs r where r.owner_id=j.owner_id and r.status in ('RUNNING','FINALIZING','LOST'))
            order by j.created_at,j.id for update of j skip locked limit 1""",{rs,_->rs.getString(1)}).firstOrNull() ?: return null
        val j=jobs.findById(id).orElseThrow()
        if(j.budgetDay==null) { // jobs accepted before the V2 migration
            val owner=owners.findById(j.ownerId).orElseThrow()
            val day=java.time.LocalDate.now(java.time.ZoneOffset.UTC)
            reserveCompute(owner,day,props.jobTimeoutSeconds);j.budgetDay=day;j.computeReservedSeconds=props.jobTimeoutSeconds
        }
        j.status="RUNNING";j.workerId=worker;j.fence++
        j.startedAt=Instant.now();j.leaseUntil=Instant.now().plusSeconds(60)
        val duration=if(j.computeReservedSeconds>0) minOf(props.jobTimeoutSeconds,j.computeReservedSeconds) else props.jobTimeoutSeconds
        j.deadlineAt=Instant.now().plusSeconds(duration);j.attemptCount++
        jdbc.update("insert into job_attempts(id,job_id,fence,worker_id,boot_id,started_at,status) values (?,?,?,?,?,now(),'RUNNING')",newId(),j.id,j.fence,worker,bootId ?: "local-test")
        return j
    }
    @Transactional fun pinImage(id:String,fence:Long,image:String):String {
        val j=jobs.locked(id) ?: error("Job missing")
        check(j.fence==fence && j.status=="RUNNING")
        if(j.runnerImage==null) j.runnerImage=image
        return j.runnerImage!!
    }
    @Transactional fun stageObject(key:String,bytes:Long) {
        jdbc.update("insert into object_gc(object_key,size_bytes,not_before) values (?,?,now()+interval '2 hours') on conflict do nothing",key,bytes)
    }
    @Transactional fun progress(id:String,fence:Long,phase:String,percent:Int,message:String?) {
        val j=jobs.locked(id) ?: return;if(j.fence!=fence || j.status !in setOf("RUNNING","FINALIZING")) return
        j.phase=phase;j.progress=percent.coerceIn(0,99);j.message=message?.take(1000);j.leaseUntil=Instant.now().plusSeconds(60)
    }
    @Transactional fun pulse(id:String,fence:Long):Boolean {
        val j=jobs.locked(id) ?: return false
        if(j.fence!=fence || j.status !in setOf("RUNNING","FINALIZING") || j.cancelRequested || j.deadlineAt?.isBefore(Instant.now())==true) return false
        j.leaseUntil=Instant.now().plusSeconds(60);return true
    }
    @Transactional fun finish(id:String,fence:Long,result:String?,outputs:List<ArtifactEntity>,error:String?) :Boolean {
        platform.locked();val j=jobs.locked(id) ?: return false
        if(j.fence!=fence || j.status!="RUNNING") return false
        if(j.cancelRequested) {terminal(j,"CANCELLED",null);return false}
        if(j.deadlineAt?.isBefore(Instant.now())==true) {terminal(j,"FAILED","Время выполнения превышено");return false}
        if(error!=null) {terminal(j,"FAILED",error);return false}
        if(outputs.sumOf { it.sizeBytes }>j.reservedBytes) {terminal(j,"FAILED","Размер результата превышает зарезервированную квоту");return false}
        for(output in outputs) {
            val valid=jdbc.queryForList("select object_key from object_gc where object_key=? and not deleting and not_before>now() for update",output.objectKey)
            check(valid.isNotEmpty()) {"Artifact staging lease expired"}
        }
        artifacts.saveAll(outputs);outputs.forEach { jdbc.update("delete from object_gc where object_key=?",it.objectKey) };j.resultJson=result;j.progress=100;terminal(j,"SUCCEEDED",null);return true
    }
    @Transactional fun finishFailure(id:String,fence:Long,outputs:List<ArtifactEntity>,error:String):Boolean {
        platform.locked();val j=jobs.locked(id) ?: return false
        if(j.fence!=fence || j.status!="RUNNING") return false
        if(j.cancelRequested) {terminal(j,"CANCELLED",null);return false}
        if(outputs.sumOf { it.sizeBytes }>j.reservedBytes) return false
        for(output in outputs) {
            val valid=jdbc.queryForList("select object_key from object_gc where object_key=? and not deleting and not_before>now() for update",output.objectKey)
            check(valid.isNotEmpty()) {"Artifact staging lease expired"}
        }
        artifacts.saveAll(outputs);outputs.forEach { jdbc.update("delete from object_gc where object_key=?",it.objectKey) };terminal(j,"FAILED",error);return true
    }
    @Transactional fun recover() {
        platform.locked();val now=Instant.now()
        (jobs.findAllByStatusOrderByCreatedAtAscIdAsc("QUEUED")+jobs.findAllByStatusOrderByCreatedAtAscIdAsc("RETRY_WAIT")).forEach { j->
            if(j.cancelRequested || owners.findById(j.ownerId).orElse(null)?.blocked!=false) terminal(j,"CANCELLED",null)
            else if(j.createdAt.isBefore(now.minusSeconds(7200))) terminal(j,"EXPIRED","Превышено время ожидания в очереди")
        }
        // LOST retains its slot until the controller confirms termination; only then may it retry.
        jobs.findAllByStatusOrderByCreatedAtAscIdAsc("RUNNING").filter { it.leaseUntil?.isBefore(now)==true }
            .forEach { it.status="LOST";it.error="Исполнитель потерял связь; слот заблокирован до подтверждения остановки" }
    }
    @Transactional fun stopped(id:String,fence:Long) {
        platform.locked();val j=jobs.locked(id) ?: return
        if(j.fence==fence && j.status in setOf("RUNNING","FINALIZING","LOST")) recoverStopped(j,"Исполнитель остановлен до завершения задачи")
    }
    @Transactional fun nodeRecoveryBegin(worker:String,boot:String,host:String=""):Boolean {
        platform.locked()
        val current=jdbc.queryForList("select * from worker_nodes where id=?",worker).firstOrNull()
        if(current!=null && current["host_id"]!="" && current["host_id"]!=host) error("Worker ID belongs to another Docker host; explicit host fencing is required")
        jdbc.update("insert into worker_nodes(id,boot_id,cpu_limit,memory_limit_bytes,capacity,active_count,last_seen_at,mode,host_id) values (?,?,0,0,0,0,now(),?,?) on conflict(id) do update set boot_id=excluded.boot_id,capacity=0,last_seen_at=now(),host_id=excluded.host_id",worker,boot,props.runnerMode,host)
        return true
    }
    @Transactional fun nodeRecoveryConfirmed(worker:String,boot:String) {
        platform.locked()
        check(jdbc.queryForObject("select count(*) from worker_nodes where id=? and boot_id=?",Long::class.java,worker,boot)==1L)
        val ids=jdbc.query("select id from queue_jobs where worker_id=? and status in ('RUNNING','FINALIZING','LOST')",{r,_->r.getString(1)},worker)
        for(id in ids) recoverStopped(jobs.findById(id).orElseThrow(),"Исполнитель перезапущен")
    }
    @Transactional fun nodePulse(node:WorkerEntity):Boolean {
        platform.locked()
        return jdbc.update("update worker_nodes set cpu_limit=?,memory_limit_bytes=?,capacity=?,active_count=?,last_seen_at=now() where id=? and boot_id=?",node.cpuLimit,node.memoryLimitBytes,node.capacity,node.activeCount,node.id,node.bootId)==1
    }
    @Transactional fun nodeStopped(worker:String,boot:String) {
        platform.locked()
        jdbc.update("update worker_nodes set capacity=0,active_count=0,last_seen_at=now()-interval '61 seconds' where id=? and boot_id=?",worker,boot)
    }
    @Transactional fun runtimeLost(id:String,fence:Long,reason:String) {
        platform.locked();val j=jobs.locked(id) ?: return
        if(j.fence==fence && j.status in setOf("RUNNING","FINALIZING")) {j.status="LOST";j.error=reason.take(2000)}
    }
    @Transactional fun retryStopped(id:String,fence:Long,reason:String) {
        platform.locked();val j=jobs.locked(id) ?: return
        if(j.fence==fence && j.status in setOf("RUNNING","FINALIZING","LOST")) recoverStopped(j,reason)
    }
    private fun recoverStopped(j:QueueJobEntity,reason:String) {
        chargeAttempt(j)
        jdbc.update("update job_attempts set status='LOST',finished_at=now(),message=? where job_id=? and fence=?",reason.take(2000),j.id,j.fence)
        if(j.cancelRequested || owners.findById(j.ownerId).orElseThrow().blocked) {terminal(j,"CANCELLED",null);return}
        if(j.attemptCount>=props.maxRetries+1 || j.computeReservedSeconds<=0) {terminal(j,"FAILED",reason);return}
        j.status="RETRY_WAIT";j.message="Повтор после сбоя исполнителя";j.error=null;j.workerId=null;j.leaseUntil=null;j.startedAt=null;j.deadlineAt=null;j.progress=0;j.availableAt=Instant.now().plusSeconds(10)
    }
    private fun reserveCompute(owner:PrincipalEntity,day:java.time.LocalDate,seconds:Long) {
        for((key,limit) in listOf(owner.id to (if(owner.kind=="GUEST") props.guestComputeSeconds else props.accountComputeSeconds),"GLOBAL" to props.globalComputeSeconds)) {
            jdbc.update("insert into compute_budgets(owner_id,day) values (?,?) on conflict do nothing",key,day)
            val total=jdbc.queryForObject("select used_seconds+reserved_seconds from compute_budgets where owner_id=? and day=?",Long::class.java,key,day)!!
            if(seconds>limit-total) limited("Суточный вычислительный бюджет исчерпан")
            jdbc.update("update compute_budgets set reserved_seconds=reserved_seconds+? where owner_id=? and day=?",seconds,key,day)
        }
    }
    private fun chargeAttempt(j:QueueJobEntity) {
        val used=if(j.startedAt!=null && j.computeReservedSeconds>0) java.time.Duration.between(j.startedAt,Instant.now()).seconds.coerceIn(1,j.computeReservedSeconds) else 0L
        if(used>0 && j.budgetDay!=null) for(key in listOf(j.ownerId,"GLOBAL")) jdbc.update("update compute_budgets set reserved_seconds=greatest(0,reserved_seconds-?),used_seconds=used_seconds+? where owner_id=? and day=?",used,used,key,j.budgetDay)
        j.computeReservedSeconds-=used;j.startedAt=null
    }
    private fun terminal(j:QueueJobEntity,status:String,error:String?) {
        val started=j.startedAt;chargeAttempt(j);j.startedAt=started
        if(j.computeReservedSeconds>0 && j.budgetDay!=null) for(key in listOf(j.ownerId,"GLOBAL")) jdbc.update("update compute_budgets set reserved_seconds=greatest(0,reserved_seconds-?) where owner_id=? and day=?",j.computeReservedSeconds,key,j.budgetDay)
        j.computeReservedSeconds=0
        jdbc.update("update job_attempts set status=?,finished_at=now(),message=? where job_id=? and fence=?",status,error?.take(2000),j.id,j.fence)
        j.status=status;j.error=error?.take(2000);j.finishedAt=Instant.now();j.leaseUntil=null;j.reservedBytes=0
        j.expiresAt=if(owners.findById(j.ownerId).orElseThrow().kind=="GUEST") Instant.now().plusSeconds(86400) else null
    }
    private fun validate(r:JobRequest) {
        if(r.kind !in listOf("PACK","SCHEDULE") || (r.fileId==null)==(r.packJobId==null)) bad("Укажите один источник и тип PACK/SCHEDULE")
        if(r.kind=="PACK" && r.packJobId!=null) bad("Для упаковки нужен загруженный файл")
        if(!r.clearance.isFinite() || r.clearance !in 0.05..10.0 || !r.resolution.isFinite() || r.resolution !in 0.05..1.0) bad("Некорректные зазор/разрешение")
        if(r.attempts !in 1..1000000 || r.layout !in listOf("search","grid") || r.benchMode !in listOf("adaptive","full")) bad("Некорректный режим поиска")
        if(r.edgeMargin!=null && (!r.edgeMargin.isFinite() || r.edgeMargin !in 0.0..20.0)) bad("Некорректный отступ")
        if(r.nozzleClean.isNotBlank()) {
            if(!Regex("^(global|from-z=\\d+(\\.\\d+)?)(,every=[1-9]\\d*)?$").matches(r.nozzleClean)) bad("Некорректная чистка сопла")
            if(r.nozzleClean.startsWith("from-z=") && (r.nozzleClean.substringAfter('=').substringBefore(',').toDoubleOrNull()?.let { it.isFinite() && it>0 }!=true)) bad("Высота должна быть положительной")
            if(",every=" in r.nozzleClean && r.nozzleClean.substringAfter(",every=").toLongOrNull()==null) bad("Слишком большой период очистки")
        }
        fun clock(s:String):Int {val m=Regex("^(\\d{1,2}):([0-5]\\d)$").matchEntire(s) ?: bad("Формат времени ЧЧ:ММ");val h=m.groupValues[1].toInt();if(h>47) bad("Часы 0–47");return h*60+m.groupValues[2].toInt()}
        if(!(clock(r.dayStart)<clock(r.nightStart) && clock(r.nightStart)<clock(r.horizonEnd)) || r.prepMin !in 0..600 || r.turnaroundMin !in 0..600) bad("Некорректное окно времени")
    }
}
@RestController @RequestMapping("/api/v2") @Profile("server")
class QueueController(private val queue:DurableQueue,private val jobs:QueueJobRepository,private val artifacts:ArtifactRepository,
    private val store:ObjectStore,private val profiles:ProfileService,private val mapper:ObjectMapper,
    private val limits:AdmissionLimits,private val jdbc:JdbcTemplate) {
    @GetMapping("/health") fun health():Map<String,String> {jdbc.queryForObject("select 1",Int::class.java);return mapOf("status" to "ok","queue" to "postgres")}
    @GetMapping("/profiles") fun profiles()=profiles.list()
    data class Access(val jobId:String="",val token:String="")
    @PostMapping("/access") fun access(@RequestBody body:Access) { queue.grant(body.jobId,actor().id,body.token) }
    @PostMapping("/jobs") fun create(@RequestBody body:JobRequest,@RequestHeader("Idempotency-Key") key:String,request:HttpServletRequest):CreatedJob {
        limits.hit("jobs-ip:${request.remoteAddr}",30,3600)
        return queue.create(actor().id,key,body)
    }
    @GetMapping("/jobs") fun list(@RequestParam(defaultValue="0") page:Int):Any {
        if(page !in 0..10000) bad("Некорректная страница")
        val rows=jobs.findAllByOwnerId(actor().id,PageRequest.of(page,20,org.springframework.data.domain.Sort.by("createdAt").descending()))
        return mapOf("items" to rows.content.map { view(it,false) },"total" to rows.totalElements)
    }
    @GetMapping("/jobs/{id}") fun get(@PathVariable id:String,@RequestParam(defaultValue="false") debug:Boolean,
        @RequestHeader(name="X-Job-Token",required=false) token:String?)=view(queue.readable(id,actor().id,token),debug)
    @PostMapping("/jobs/{id}/share") fun share(@PathVariable id:String)=mapOf("token" to queue.share(id,actor().id))
    @DeleteMapping("/jobs/{id}") fun cancel(@PathVariable id:String) {queue.delete(id,actor().id)}
    @PostMapping("/jobs/{id}/artifacts/{artifact}/download") fun download(@PathVariable id:String,@PathVariable artifact:String,
        @RequestHeader(name="X-Job-Token",required=false) token:String?):Map<String,String> {
        val job=queue.readable(id,actor().id,token)
        val a=artifacts.findById(artifact).orElse(null)?.takeIf { it.jobId==id } ?: missing()
        val seconds=job.expiresAt?.let { (it.epochSecond-Instant.now().epochSecond).coerceIn(1,300) } ?: 300
        val url=store.signedGet(a.objectKey,a.downloadName,seconds) ?: "/api/v2/jobs/$id/artifacts/$artifact/file"
        return mapOf("url" to url,"filename" to a.downloadName)
    }
    @GetMapping("/jobs/{id}/artifacts/{artifact}/file") fun localDownload(@PathVariable id:String,@PathVariable artifact:String):org.springframework.http.ResponseEntity<ByteArray> {
        queue.readable(id,actor().id,null)
        val a=artifacts.findById(artifact).orElse(null)?.takeIf { it.jobId==id } ?: missing()
        val tmp=Files.createTempFile("goat-download-", ".bin")
        try { store.get(a.objectKey,tmp);return org.springframework.http.ResponseEntity.ok().header("Content-Disposition",org.springframework.http.ContentDisposition.attachment().filename(a.downloadName,Charsets.UTF_8).build().toString()).body(Files.readAllBytes(tmp)) }
        finally { Files.deleteIfExists(tmp) }
    }
    private fun view(j:QueueJobEntity,debug:Boolean):Map<String,Any?> {
        val ahead=if(j.status in setOf("QUEUED","RETRY_WAIT")) jdbc.queryForObject("select count(*) from queue_jobs where status in ('QUEUED','RETRY_WAIT') and (created_at,id)<(?,?)",Long::class.java,java.sql.Timestamp.from(j.createdAt),j.id)!! else null
        val capacity=jdbc.queryForObject("select coalesce(sum(capacity),0) from worker_nodes where last_seen_at>now()-interval '30 seconds'",Int::class.java)!!
        val configured=jdbc.queryForObject("select case when paused then 0 else max_parallel end from platform_config where id=1",Int::class.java)!!
        val slots=minOf(capacity,configured)
        val stats=jdbc.queryForMap("""select count(*) as n, percentile_cont(0.5) within group(order by extract(epoch from finished_at-started_at)) as median
            from (select started_at,finished_at from queue_jobs where status='SUCCEEDED' and kind=? and profile_id=? and started_at is not null
            order by finished_at desc limit 50) sample""",j.kind,j.profileId)
        val running=jdbc.queryForObject("select count(*) from queue_jobs where status in ('RUNNING','FINALIZING','LOST')",Long::class.java)!!
        val estimate=if(ahead!=null && slots>0 && (stats["n"] as Number).toInt()>=5)
            (((ahead+running).toDouble()/slots)*(stats["median"] as Number).toDouble()).toLong().coerceAtLeast(0) else null
        val result=j.resultJson?.let { mapper.readTree(it) }
        if(j.kind=="SCHEDULE") result?.path("jobs")?.forEach { row ->
            (row as? com.fasterxml.jackson.databind.node.ObjectNode)?.apply {
                put("startClock",com.goatpacker.web.service.ClockFormat.format(path("startSeconds").asLong()))
                put("endClock",com.goatpacker.web.service.ClockFormat.format(path("endSeconds").asLong()))
            }
        }
        return mapOf("canManage" to (j.ownerId==actor().id),"jobId" to j.id,"kind" to j.kind.lowercase(),"status" to j.status,"phase" to j.phase,"progressPercent" to j.progress,
            "phaseMessage" to j.message,"error" to (if(debug) j.error else j.error?.substringBefore(com.goatpacker.web.job.CliErrorTranslator.DETAILS_SEPARATOR)?.take(500)),"createdAt" to j.createdAt,"finishedAt" to j.finishedAt,"expiresAt" to j.expiresAt,
            "originalName" to j.originalName,"queuePosition" to ahead?.plus(1),"estimatedWaitSeconds" to estimate,"estimatedWaitFromSeconds" to estimate?.div(2),"estimatedWaitToSeconds" to estimate?.let { it*2+10 },
            "estimateReason" to (if(slots==0) "Нет доступных исполнителей или очередь на паузе" else if(estimate==null) "Недостаточно измерений для надёжной оценки" else "Приблизительно по медиане последних задач; фактическое время может отличаться"),
            "artifacts" to artifacts.findAllByJobId(j.id).filter { debug || !it.debug }.map { mapOf("id" to it.id,"name" to it.downloadName,"sizeBytes" to it.sizeBytes,"downloadUrl" to "/api/v2/jobs/${j.id}/artifacts/${it.id}/download") },
            "pack" to if(j.kind=="PACK") result else null,"schedule" to if(j.kind=="SCHEDULE") result else null)
    }
}
