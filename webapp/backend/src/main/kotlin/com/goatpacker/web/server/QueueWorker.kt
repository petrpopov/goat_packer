package com.goatpacker.web.server

import com.fasterxml.jackson.databind.ObjectMapper
import com.fasterxml.jackson.module.kotlin.readValue
import com.goatpacker.web.config.AppProperties
import com.goatpacker.web.job.*
import jakarta.annotation.PreDestroy
import org.slf4j.LoggerFactory
import org.springframework.boot.autoconfigure.condition.ConditionalOnProperty
import org.springframework.context.annotation.Profile
import org.springframework.scheduling.annotation.Scheduled
import org.springframework.stereotype.Component
import java.nio.channels.FileChannel
import java.nio.channels.FileLock
import java.nio.file.*
import java.time.*
import java.util.concurrent.*
import kotlin.math.floor

@Component @Profile("server") @ConditionalOnProperty(name=["platform.worker-enabled"],havingValue="true")
class QueueWorker(private val queue:DurableQueue,private val jobs:QueueJobRepository,private val blobs:BlobRepository,
    private val nodes:WorkerRepository,private val store:ObjectStore,private val runner:JobRunner,
    private val launcher:NativeLauncher,private val app:AppProperties,private val props:PlatformProperties,private val mapper:ObjectMapper) {
    private val log=LoggerFactory.getLogger(javaClass)
    private val bootId=newId()
    private val capacity=minOf(floor(props.executionCpuBudget/props.workerCpus).toInt(),(props.executionMemoryMib/props.workerMemoryMib).toInt(),64).coerceAtLeast(0)
    private val pool=Executors.newFixedThreadPool(capacity.coerceAtLeast(1))
    private val live=ConcurrentHashMap<String,Job>()
    private val fences=ConcurrentHashMap<String,Long>()
    private val cleanupPending=ConcurrentHashMap.newKeySet<String>()
    @Volatile private var closing=false
    @Volatile private var registered=false
    private var images:Map<String,String> = emptyMap()
    private var host=""
    private val root=app.jobsRootPath.resolve(props.workerId)
    private var lockChannel:FileChannel?=null
    private var workerLock:FileLock?=null

    /** An OS file lock fences paused/duplicate JVMs on this host, not just DB heartbeats.
     * Root MUST be a persistent per-host scratch volume shared by controllers with this ID. */
    private fun register() {
        require(Regex("[a-zA-Z0-9_-]{1,64}").matches(props.workerId))
        Files.createDirectories(root)
        Files.deleteIfExists(root.resolve("ready"))
        if(workerLock==null) {
            lockChannel=FileChannel.open(root.resolve("controller.lock"),StandardOpenOption.CREATE,StandardOpenOption.WRITE)
            try { workerLock=lockChannel!!.tryLock() ?: error("Another controller holds this worker's filesystem lock") }
            catch(ex:Exception) {lockChannel?.close();lockChannel=null;throw ex}
        }
        images=launcher.qualify();host=launcher.hostIdentity()
        check(queue.nodeRecoveryBegin(props.workerId,bootId,host))
        launcher.recoverNode() // confirm all old native processes are dead BEFORE releasing their DB slots
        queue.nodeRecoveryConfirmed(props.workerId,bootId)
        Files.list(root).use { entries -> entries.filter { Files.isDirectory(it,LinkOption.NOFOLLOW_LINKS) && runCatching {java.util.UUID.fromString(it.fileName.toString())}.isSuccess }.forEach { it.toFile().deleteRecursively() } }
        registered=true
        log.info("event=worker_ready workerId={} bootId={} capacity={} cpu={} memoryMiB={} runtimes={}",props.workerId,bootId,capacity,props.executionCpuBudget,props.executionMemoryMib,images)
    }
    @Scheduled(fixedDelay=2000,initialDelay=3000)
    fun dispatch() {
        if(closing) return
        if(!registered) register()
        val effective=if(Files.getFileStore(root).usableSpace>props.outputReservationBytes+props.maxUploadBytes+512L*1024*1024) capacity else 0
        if(!queue.nodePulse(WorkerEntity(props.workerId,bootId,props.executionCpuBudget,props.executionMemoryMib*1024*1024,effective,live.size,Instant.now(),props.runnerMode,host))) {
            closing=true;live.values.forEach(::stopNative);return
        }
        Files.writeString(root.resolve("ready"),Instant.now().toString())
        queue.recover()
        while(!closing && live.size<effective) {
            val j=queue.claim(props.workerId,bootId) ?: break
            val job=Job(j.id,JobKind.valueOf(j.kind),root.resolve(j.id).resolve(j.fence.toString()),OffsetDateTime.now())
            job.runtimeFence=j.fence
            job.runtimeTimeoutSeconds=Duration.between(Instant.now(),j.deadlineAt).seconds.coerceAtLeast(1)
            live[j.id]=job;fences[j.id]=j.fence
            pool.submit { run(j,job) }
        }
    }
    @Scheduled(fixedDelay=5000,initialDelay=5000)
    fun heartbeat() {
        live.forEach { (id,job) ->
            if(id in cleanupPending) {
                if(stopNative(job)) runCatching {queue.retryStopped(id,fences[id] ?: -1,"Восстановление после сбоя исполнителя");release(job)}
                return@forEach
            }
            val safe=runCatching {queue.pulse(id,fences[id] ?: -1)}.getOrDefault(false)
            if(!safe) {job.cancel();stopNative(job)}
        }
    }
    private fun stopNative(job:Job):Boolean = runCatching {
        launcher.stop(job);job.process?.let(JobProcessKiller::kill);true
    }.getOrElse { log.error("event=runner_cleanup_failed jobId={} reason={}",job.id,it.message);false }

    private fun run(record:QueueJobEntity,job:Job) {
        org.slf4j.MDC.put("jobId",record.id);org.slf4j.MDC.put("attemptFence",record.fence.toString())
        val uploaded=mutableListOf<ArtifactEntity>()
        var retryReason:String?=null
        try {
            Files.createDirectories(job.directory)
            val blob=blobs.findById(record.inputBlobId).orElseThrow()
            store.get(blob.objectKey,job.directory.resolve("input.3mf"))
            if(Regex("[0-9a-f]{64}").matches(blob.sha256)) {
                val digest=java.security.MessageDigest.getInstance("SHA-256")
                Files.newInputStream(job.directory.resolve("input.3mf")).use { input-> val buffer=ByteArray(65536);while(true){val n=input.read(buffer);if(n<0)break;digest.update(buffer,0,n)} }
                check(digest.digest().joinToString(""){"%02x".format(it)}==blob.sha256) {"Input checksum mismatch"}
            }
            val profile=mapper.readTree(record.profileJson) as com.fasterxml.jackson.databind.node.ObjectNode
            if(props.runnerMode=="docker") {
                val kind=profile.path("slicer_kind").asText()
                job.runtimeImage=queue.pinImage(record.id,record.fence,images[kind] ?: error("Unsupported runtime"))
                profile.put("slicer_path",if(kind=="bambu") "/opt/slicers/bambu/AppRun" else "/opt/slicers/orca/AppRun")
            }
            Files.writeString(job.directory.resolve("profile.json"),mapper.writeValueAsString(profile))
            var last=0L
            job.onUpdate={snap -> if(System.nanoTime()-last>TimeUnit.SECONDS.toNanos(1) || snap.status!=JobStatus.RUNNING) {
                last=System.nanoTime();queue.progress(record.id,record.fence,snap.phase.jsonValue(),snap.progressPercent,snap.phaseMessage)
            }}
            if(!queue.pulse(record.id,record.fence)) {job.cancel();return}
            val r=mapper.readValue<JobRequest>(record.paramsJson)
            log.info("event=job_started jobId={} fence={} kind={} image={} cpu={} memoryMiB={}",record.id,record.fence,record.kind,job.runtimeImage,props.workerCpus,props.workerMemoryMib)
            if(record.kind=="PACK") runner.runPackBlocking(job,PackParams(record.profileId,Path.of("profile.json"),r.clearance,r.resolution,r.attempts,2400,20270831,r.edgeMargin,r.layout,r.nozzleClean,
                timeWindowHours=r.timeWindowHours,amsCorrection=r.amsCorrection,changeSeconds=r.changeSeconds,timeMarginPct=r.timeMarginPct,
                supportOverlapMm=r.supportOverlapMm))
            else if(record.kind=="CALIBRATION") runner.runCalibrationBlocking(job,CalibrationParams(record.profileId,Path.of("profile.json"),r.variant))
            else runner.runScheduleBlocking(job,ScheduleParams(record.profileId,Path.of("profile.json"),ScheduleSource.Uploaded,r.dayStart,r.nightStart,r.horizonEnd,r.turnaroundMin,r.prepMin,r.benchMode,
                amsCorrection=r.amsCorrection,changeSeconds=r.changeSeconds))
            if(!stopNative(job)) {retryReason="Остановка контейнера не подтверждена";return}
            if(job.cancelled) {queue.finish(record.id,record.fence,null,emptyList(),"Задача остановлена по лимиту или отменена");return}
            val result=when(record.kind) {
                "PACK" -> mapper.writeValueAsString(job.packResult)
                "CALIBRATION" -> mapper.writeValueAsString(job.calibrationResult)
                else -> mapper.writeValueAsString(job.scheduleResult)
            }
            var bytes=0L
            for((name,path) in JobArtifacts.listExisting(job.kind,job.directory)) {
                val size=Files.size(path);bytes+=size;check(bytes<=record.reservedBytes) {"Output exceeds reserved quota"}
                val base=safeName(record.originalName).replace(Regex("(?i)\\.3mf$"),"").take(80)
                val slug=profile.path("download_slug").asText(record.profileId).replace(Regex("[^a-zA-Z0-9_-]"),"_").take(40)
                val count=job.packResult?.objectCount ?: job.scheduleResult?.totalObjects ?: 0
                // Калибровочные файлы сохраняют свои имена: instructions.md ссылается на calib_A/calib_B.
                val download=if(record.kind=="CALIBRATION") name
                    else if(name.endsWith(".3mf")) "${base}_${slug}_${count}_${if(record.kind=="PACK") "packed" else "schedule"}.3mf" else name
                val key="jobs/${record.id}/${record.fence}/${newId()}/$name"
                queue.stageObject(key,size);store.put(key,path)
                uploaded+=ArtifactEntity(jobId=record.id,name=name,downloadName=download,objectKey=key,sizeBytes=size,debug=(name.endsWith(".json") && name!="calibration.json") || name.endsWith(".txt"))
            }
            val outputs=if(job.current.status==JobStatus.SUCCEEDED) uploaded else uploaded.filter {it.debug}
            val committed=if(job.current.status==JobStatus.SUCCEEDED) queue.finish(record.id,record.fence,result,outputs,null)
                else queue.finishFailure(record.id,record.fence,outputs,job.current.error ?: "Ошибка обработки")
            log.info("event=job_finished jobId={} state={} published={}",record.id,job.current.status,committed)
        } catch(ex:Exception) {
            log.error("event=job_failed jobId={} reason={}",record.id,ex.message)
            retryReason="Временная ошибка инфраструктуры: ${ex.message?.take(500)}"
        } finally {
            if(stopNative(job)) {
                val settled=runCatching {
                    if(retryReason!=null) queue.retryStopped(record.id,record.fence,retryReason!!)
                    else queue.stopped(record.id,record.fence)
                }.isSuccess
                // Never delete staged outputs here: a DB commit can succeed while its reply is lost.
                // The durable GC marker is removed atomically with publication, and owns cleanup.
                if(settled) release(job) else cleanupPending.add(job.id)
            } else {
                cleanupPending.add(job.id)
                runCatching {queue.runtimeLost(record.id,record.fence,"Runtime cleanup pending; slot remains reserved")}
            }
            org.slf4j.MDC.remove("jobId");org.slf4j.MDC.remove("attemptFence")
        }
    }
    private fun release(job:Job) {job.directory.toFile().deleteRecursively();live.remove(job.id);fences.remove(job.id);cleanupPending.remove(job.id)}
    @PreDestroy fun stop() {
        closing=true;Files.deleteIfExists(root.resolve("ready"));live.values.forEach {it.cancel();stopNative(it)}
        pool.shutdown();pool.awaitTermination(20,TimeUnit.SECONDS);pool.shutdownNow()
        if(live.values.all {stopNative(it)}) {
            if(registered) runCatching {queue.nodeStopped(props.workerId,bootId)}
            workerLock?.release();lockChannel?.close()
        }
    }
}
