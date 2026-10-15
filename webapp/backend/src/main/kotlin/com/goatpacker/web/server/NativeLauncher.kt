package com.goatpacker.web.server

import com.fasterxml.jackson.databind.ObjectMapper
import com.goatpacker.web.job.Job
import org.apache.commons.compress.archivers.tar.TarArchiveInputStream
import org.springframework.context.annotation.Profile
import org.springframework.stereotype.Component
import java.nio.file.Files
import java.nio.file.LinkOption
import java.nio.file.Path
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.TimeUnit

/** Only the trusted worker controls Docker. Untrusted code receives a bounded tmpfs, not a
 * writable host mount, network, daemon socket, credentials, or another job's directories. */
@Component @Profile("server")
class NativeLauncher(private val props:PlatformProperties,private val json:ObjectMapper) {
    private val containers=ConcurrentHashMap<String,String>()
    private val volumes=ConcurrentHashMap<String,String>()
    private val images=ConcurrentHashMap<String,String>()
    fun image(kind:String):String = if(kind=="bambu") props.runnerBambuImage else props.runnerOrcaImage
    fun qualify():Map<String,String> {
        if(props.runnerMode=="local") {check(props.allowLocalRunner && !props.production);return emptyMap()}
        return listOf("bambu","orca").associateWith { kind ->
            val ref=image(kind);check(ref.isNotBlank()) {"Runner image missing: $kind"}
            val id=runCommand(listOf("docker","image","inspect","--format","{{.Id}}",ref)).trim()
            check(id.startsWith("sha256:")); id
        }
    }
    fun hostIdentity():String = if(props.runnerMode=="local") "local" else runCommand(listOf("docker","info","--format","{{.ID}}")).trim()
    fun recoverNode() {
        if(props.runnerMode=="local") return
        val ids=runCommand(listOf("docker","ps","-aq","--filter","label=goat.worker=${props.workerId}")).lines().filter(String::isNotBlank)
        for(id in ids) remove(id)
        val stale=runCommand(listOf("docker","volume","ls","-q","--filter","label=goat.worker=${props.workerId}")).lines().filter(String::isNotBlank)
        for(volume in stale) runCommand(listOf("docker","volume","rm",volume))
    }
    fun start(job:Job,command:List<String>):Process {
        if(props.runnerMode=="local") {
            check(props.allowLocalRunner && !props.production) {"Local runner forbidden in production"}
            return ProcessBuilder(command).directory(job.directory.toFile()).redirectErrorStream(true)
                .also {it.environment()["RAYON_NUM_THREADS"]=props.workerCpus.toInt().coerceAtLeast(1).toString()}.start()
        }
        val kind=json.readTree(job.directory.resolve("profile.json").toFile()).path("slicer_kind").asText()
        val ref=job.runtimeImage ?: image(kind);check(ref.isNotBlank())
        val name="goat-${job.id}-${job.runtimeFence}"
        val volume="$name-work"
        runCommand(listOf("docker","volume","create","--label","goat.worker=${props.workerId}","--driver","local",
            "--opt","type=tmpfs","--opt","device=tmpfs","--opt","o=size=${props.scratchLimitBytes},uid=1000,gid=1000,mode=0700,nosuid,nodev",volume))
        volumes[job.id]=volume;images[job.id]=ref
        val create=listOf("docker","create","--platform","linux/amd64","--name",name,
            "--label","goat.worker=${props.workerId}","--label","goat.job=${job.id}","--label","goat.fence=${job.runtimeFence}",
            "--network","none","--read-only","--cap-drop","ALL","--security-opt","no-new-privileges",
            "--pids-limit","256","--cpus",props.workerCpus.toString(),"--memory","${props.workerMemoryMib}m",
            "--memory-swap","${props.workerMemoryMib}m","--user","1000:1000","--init",
            "--tmpfs","/tmp:rw,noexec,nosuid,nodev,size=64m",
            "--mount","type=volume,src=$volume,dst=/work,volume-nocopy",
            "--workdir","/work","--env","HOME=/work/home","--env","RAYON_NUM_THREADS=${props.workerCpus.toInt().coerceAtLeast(1)}",
            "--env","GOAT_LOG_FORMAT=json",ref,"/usr/bin/sleep","infinity")
        try {
            val id=runCommand(create).trim();containers[job.id]=id
            runCommand(listOf("docker","start",id))
            for(file in listOf("input.3mf","profile.json")) {
                val source=job.directory.resolve(file);check(Files.isRegularFile(source,LinkOption.NOFOLLOW_LINKS))
                val transfer=ProcessBuilder("docker","exec","-i",id,"/bin/sh","-c","cat > /work/$file")
                    .redirectInput(source.toFile()).redirectOutput(ProcessBuilder.Redirect.DISCARD).redirectError(ProcessBuilder.Redirect.DISCARD).start()
                if(!transfer.waitFor(30,TimeUnit.SECONDS)) {transfer.destroyForcibly();error("Input transfer timed out")}
                check(transfer.exitValue()==0) {"Input transfer failed"}
            }
            // Independent timeout survives a disconnected JVM or a dead control plane.
            val args=listOf("docker","exec",id,"/usr/bin/timeout","--signal=TERM","--kill-after=5s",job.runtimeTimeoutSeconds.toString(),
                "/usr/local/bin/${Path.of(command.first()).fileName}")+command.drop(1)
            return ProcessBuilder(args).redirectErrorStream(true).start()
        } catch(ex:Exception) {stop(job);throw ex}
    }
    /** Freeze all descendants before reading files, so export has no symlink/size TOCTOU. */
    fun collect(job:Job) {
        if(props.runnerMode=="local") return
        val id=containers[job.id] ?: error("Runner not tracked")
        runCommand(listOf("docker","pause",id))
        var remaining=props.outputReservationBytes
        for(name in listOf("packed.3mf","packed.verification.json","work/placements.json","schedule.csv","plates.3mf","report.json")) {
            val target=job.directory.resolve(name);Files.createDirectories(target.parent)
            val process=ProcessBuilder("docker","run","--rm","--platform","linux/amd64","--name","goat-export-${job.id}",
                "--label","goat.worker=${props.workerId}","--network","none","--read-only","--cap-drop","ALL",
                "--security-opt","no-new-privileges","--pids-limit","32","--memory","128m","--cpus","0.5","--user","1000:1000",
                "--mount","type=volume,src=${volumes[job.id]},dst=/export,readonly,volume-nocopy",
                images[job.id]!!,"/bin/tar","-C","/export","-cf","-","--",name).redirectError(ProcessBuilder.Redirect.DISCARD).start()
            try {
                java.util.concurrent.CompletableFuture.runAsync {
                TarArchiveInputStream(process.inputStream).use { tar ->
                    val entry=tar.nextEntry ?: return@use
                    check(entry.isFile && !entry.isSymbolicLink && !entry.isLink && entry.size in 0..remaining) {"Invalid/oversized runner artifact"}
                    Files.newOutputStream(target).use { out ->
                        val buffer=ByteArray(65536);var copied=0L
                        while(true) {val n=tar.read(buffer);if(n<0) break;copied+=n;check(copied<=remaining);out.write(buffer,0,n)}
                        check(copied==entry.size);remaining-=copied
                    }
                    check(tar.nextEntry==null) {"Expected one regular artifact"}
                }
                }.get(30,TimeUnit.SECONDS)
                if(!process.waitFor(15,TimeUnit.SECONDS)) error("Artifact copy timed out")
            } finally {if(process.isAlive) process.destroyForcibly()}
        }
        stop(job)
    }
    fun stop(job:Job) {
        // An export helper is trusted but may survive a disconnected Docker CLI.
        if(props.runnerMode=="docker" && (containers.containsKey(job.id) || volumes.containsKey(job.id))) remove("goat-export-${job.id}")
        containers[job.id]?.let {id -> remove(id);containers.remove(job.id,id)}
        volumes[job.id]?.let {volume -> runCommand(listOf("docker","volume","rm",volume));volumes.remove(job.id,volume)}
        images.remove(job.id)
    }
    fun tracked(job:Job)=containers.containsKey(job.id)
    private fun remove(id:String) {
        val p=ProcessBuilder("docker","rm","-f",id).redirectOutput(ProcessBuilder.Redirect.DISCARD).redirectError(ProcessBuilder.Redirect.DISCARD).start()
        if(!p.waitFor(15,TimeUnit.SECONDS)) {p.destroyForcibly();error("Docker termination timed out")}
        if(p.exitValue()!=0) {
            val exists=runCommand(listOf("docker","ps","-a","--format","{{.ID}} {{.Names}}")).lines().filter(String::isNotBlank).any { it.substringBefore(' ').let {short -> id.startsWith(short)} || it.substringAfter(' ')==id }
            check(!exists) {"Native container termination not confirmed"}
        }
    }
    private fun runCommand(args:List<String>):String {
        val p=ProcessBuilder(args).redirectErrorStream(true).start()
        val output=java.util.concurrent.CompletableFuture.supplyAsync {p.inputStream.readNBytes(65536).toString(Charsets.UTF_8)}
        if(!p.waitFor(60,TimeUnit.SECONDS)) {p.destroyForcibly();error("Runtime command timed out")}
        val text=output.get(5,TimeUnit.SECONDS);check(p.exitValue()==0) {"Runtime error: ${text.take(1000)}"};return text
    }
}
