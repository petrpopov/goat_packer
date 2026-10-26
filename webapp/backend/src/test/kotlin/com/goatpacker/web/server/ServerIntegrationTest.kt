package com.goatpacker.web.server

import com.fasterxml.jackson.databind.ObjectMapper
import com.goatpacker.web.descriptorProject3mf
import jakarta.servlet.http.Cookie
import org.junit.jupiter.api.Test
import org.junit.jupiter.api.BeforeEach
import org.junit.jupiter.api.Assertions.*
import org.springframework.beans.factory.annotation.Autowired
import org.springframework.boot.test.autoconfigure.web.servlet.AutoConfigureMockMvc
import org.springframework.boot.test.context.SpringBootTest
import org.springframework.http.HttpMethod
import org.springframework.jdbc.core.JdbcTemplate
import org.springframework.test.context.*
import org.springframework.test.web.servlet.MockMvc
import org.springframework.test.web.servlet.request.MockMvcRequestBuilders.*
import org.testcontainers.containers.PostgreSQLContainer
import java.nio.file.Files
import java.util.concurrent.*
import java.util.concurrent.atomic.AtomicInteger

@SpringBootTest @AutoConfigureMockMvc @ActiveProfiles("server")
class ServerIntegrationTest(@Autowired val mvc:MockMvc,@Autowired val json:ObjectMapper,
    @Autowired val jdbc:JdbcTemplate,@Autowired val queue:DurableQueue,@Autowired val jobs:QueueJobRepository,
    @Autowired val principals:PrincipalRepository,@Autowired val blobs:BlobRepository,@Autowired val nodes:WorkerRepository,
    @Autowired val store:ObjectStore,@Autowired val runner:com.goatpacker.web.job.JobRunner,@Autowired val launcher:NativeLauncher,
    @Autowired val app:com.goatpacker.web.config.AppProperties,@Autowired val props:PlatformProperties) {
    companion object {
        class Pg:PostgreSQLContainer<Pg>("postgres:17.0")
        private val pg=Pg().apply { start() }
        private val root=Files.createTempDirectory("goat-server-test-")
        private val ids=AtomicInteger(0)
        @JvmStatic @DynamicPropertySource fun properties(r:DynamicPropertyRegistry) {
            r.add("spring.datasource.url",pg::getJdbcUrl);r.add("spring.datasource.username",pg::getUsername);r.add("spring.datasource.password",pg::getPassword)
            r.add("platform.storage-mode"){"file"};r.add("platform.local-storage-root"){root.resolve("objects").toString()}
            r.add("platform.admin-login"){"admin"};r.add("platform.admin-password"){"Test-admin-secret-123"}
            r.add("platform.worker-enabled"){false};r.add("platform.secure-cookies"){false}
            r.add("platform.runner-mode"){"local"};r.add("platform.allow-local-runner"){true}
            r.add("app.packer-bin"){com.goatpacker.web.FakeBins.install(root,"fake-packer.sh")}
            r.add("app.scheduler-bin"){com.goatpacker.web.FakeBins.install(root,"fake-scheduler.sh")}
            r.add("app.calibration-bin"){com.goatpacker.web.FakeBins.install(root,"fake-calibration.sh")}
            r.add("app.jobs-root"){root.resolve("work").toString()};r.add("app.profiles-dir"){"../../profiles"}
        }
    }
    inner class Client {
        private val ip="10.10.0.${ids.incrementAndGet()}"
        private val cookies=linkedMapOf<String,Cookie>()
        init {call("GET","/api/v2/auth/csrf")}
        fun call(method:String,path:String,body:Any?=null,expected:Int=200,headers:Map<String,String> = emptyMap()):com.fasterxml.jackson.databind.JsonNode {
            val builder=request(HttpMethod.valueOf(method),path).with { it.remoteAddr=ip;it }
            if(cookies.isNotEmpty()) builder.cookie(*cookies.values.toTypedArray())
            if(method !in listOf("GET","HEAD")) cookies["XSRF-TOKEN"]?.let {builder.header("X-XSRF-TOKEN",it.value)}
            headers.forEach { (key,value)->builder.header(key,value) }
            if(body is ByteArray) builder.contentType("application/octet-stream").content(body)
            else if(body!=null) builder.contentType("application/json").content(json.writeValueAsBytes(body))
            val res=mvc.perform(builder).andReturn().response
            res.getHeaders("Set-Cookie").forEach { value->val pair=value.substringBefore(';').split('=',limit=2);cookies[pair[0]]=Cookie(pair[0],pair[1]) }
            assertEquals(expected,res.status,"$method $path: ${res.contentAsString}")
            return if(res.contentAsString.isBlank()) json.createObjectNode() else json.readTree(res.contentAsString)
        }
        fun guest()=call("POST","/api/v2/auth/guest")
        fun login(name:String="admin",pass:String="Test-admin-secret-123",expected:Int=200)=call("POST","/api/v2/auth/login",mapOf("login" to name,"password" to pass),expected)
        fun upload(printer:String="Bambu Lab P1S",profile:String="bambu_p1s"):String {
            val data=descriptorProject3mf(printer)
            val id=call("POST","/api/v2/uploads",mapOf("name" to "cat.3mf","sizeBytes" to data.size))["fileId"].asText()
            assertEquals(profile,call("PUT","/api/v2/uploads/$id/content",data,headers=mapOf("Origin" to "http://localhost:3007"))["profileId"].asText())
            return id
        }
        fun job(file:String,key:String=newId(),extra:Map<String,Any> = emptyMap(),expected:Int=200)=call("POST","/api/v2/jobs",mapOf("fileId" to file,"profileId" to "bambu_p1s")+extra,expected,headers=mapOf("Idempotency-Key" to key))
    }
    @BeforeEach fun clearQueue() {
        jdbc.update("delete from compute_budgets");jdbc.update("delete from bot_challenges");jdbc.update("delete from object_gc");jdbc.update("delete from worker_nodes");jdbc.update("delete from artifacts");jdbc.update("delete from queue_jobs");jdbc.update("delete from uploads");jdbc.update("delete from blobs")
        jdbc.update("update platform_config set max_parallel=1,paused=false where id=1")
    }
    @Test fun `server denies legacy endpoints admin to guests and mutation without csrf`() {
        val guest=Client();guest.guest()
        guest.call("GET","/api/v2/admin/users",expected=403)
        guest.call("GET","/api/profiles",expected=403)
        assertEquals(403,mvc.perform(post("/api/v2/auth/guest")).andReturn().response.status)
    }
    @Test fun `administrator sees guests blocks access and cannot block self`() {
        val guest=Client();val id=guest.guest()["id"].asText()
        val admin=Client();val adminId=admin.login()["id"].asText()
        assertTrue(admin.call("GET","/api/v2/admin/users?kind=GUEST")["items"].any { it["id"].asText()==id })
        admin.call("POST","/api/v2/admin/users/$id/block",mapOf("blocked" to true,"reason" to "abuse report"))
        guest.call("GET","/api/v2/auth/me",expected=403)
        admin.call("POST","/api/v2/admin/users/$adminId/block",mapOf("blocked" to true,"reason" to "test guard"),400)
        assertTrue(admin.call("GET","/api/v2/admin/audit").any { it["action"].asText()=="ACCESS_BLOCKED" })
    }
    @Test fun `password reset revokes sessions cannot be reused and never reveals password`() {
        val user=Client();user.guest();val name="user${ids.incrementAndGet()}"
        val id=user.call("POST","/api/v2/auth/register",mapOf("login" to name,"password" to "Original-password-123","role" to "ADMIN"))["id"].asText()
        assertEquals("USER",user.call("GET","/api/v2/auth/me")["role"].asText())
        val admin=Client();admin.login()
        val response=admin.call("POST","/api/v2/admin/users/$id/password-reset",mapOf("reason" to "User requested reset"))
        user.call("GET","/api/v2/auth/me",expected=401)
        val token=response["token"].asText();assertTrue(token.length>=40)
        user.call("POST","/api/v2/auth/reset",mapOf("token" to token,"password" to "Replacement-secret-123"))
        user.call("POST","/api/v2/auth/reset",mapOf("token" to token,"password" to "Replacement-secret-456"),400)
        user.login(name,"Replacement-secret-123")
        assertFalse(admin.call("GET","/api/v2/admin/users?query=$name").toString().contains("passwordHash"))
    }
    @Test fun `dedup queue ownership idempotency capabilities and profile snapshot`() {
        val a=Client();a.guest();val b=Client();b.guest()
        val file=a.upload();b.upload()
        assertEquals(1,jdbc.queryForObject("select count(*) from blobs",Int::class.java))
        val key=newId();val created=a.job(file,key);val id=created["jobId"].asText()
        assertEquals(id,a.job(file,key)["jobId"].asText())
        assertEquals(1,jdbc.queryForObject("select count(*) from queue_jobs",Int::class.java))
        b.call("GET","/api/v2/jobs/$id",expected=404)
        b.call("GET","/api/v2/jobs/$id",headers=mapOf("X-Job-Token" to created["readToken"].asText()))
        b.call("DELETE","/api/v2/jobs/$id",expected=404)
        assertEquals(1,a.call("GET","/api/v2/jobs/$id")["queuePosition"].asInt())
        assertTrue(jobs.findById(id).orElseThrow().profileJson.contains("Bambu Lab P1S"))
        a.call("DELETE","/api/v2/jobs/$id")
        assertEquals("CANCELLED",a.call("GET","/api/v2/jobs/$id")["status"].asText())
    }
    @Test fun `concurrent workers cannot exceed global slots or claim same job`() {
        val a=Client();a.guest();a.job(a.upload())
        val b=Client();b.guest();b.job(b.upload())
        (1..4).forEach {nodes.save(WorkerEntity("worker-$it",newId(),2.0,1536L*1024*1024,1,0))}
        nodes.save(WorkerEntity("next-worker",newId(),2.0,1536L*1024*1024,1,0))
        val executor=Executors.newFixedThreadPool(4)
        try {
            val claims=(1..4).map { executor.submit<QueueJobEntity?> {queue.claim("worker-$it")} }.mapNotNull { it.get(10,TimeUnit.SECONDS) }
            assertEquals(1,claims.size)
            val job=claims.single()
            assertFalse(queue.finish(job.id,job.fence+1,"{}",emptyList(),null))
            assertFalse(queue.finish(job.id,job.fence,null,emptyList(),"test failure"))
            assertEquals("FAILED",jobs.findById(job.id).orElseThrow().status)
            assertNotNull(queue.claim("next-worker"))
        } finally {executor.shutdownNow()}
    }
    @Test fun `admin system limits pause admission without pretending to kill running jobs`() {
        val admin=Client();admin.login()
        admin.call("POST","/api/v2/admin/system",mapOf("maxParallel" to 0,"paused" to true,"reason" to "maintenance"))
        val state=admin.call("GET","/api/v2/admin/system")
        assertEquals(0,state["configuredParallelism"].asInt());assertTrue(state["paused"].asBoolean())
        assertNull(queue.claim("none"))
    }
    @Test fun `complete durable job workflow publishes named files and hides diagnostics`() {
        val client=Client();client.guest()
        val file=client.upload()
        // Окно/смена/запас вне диапазона отсекаются при постановке, до очереди.
        for(bad in listOf(mapOf("timeWindowHours" to 0.1),mapOf("changeSeconds" to 601),mapOf("timeMarginPct" to 51)))
            client.job(file,extra=bad,expected=400)
        val id=client.job(file,extra=mapOf("timeWindowHours" to 12,"amsCorrection" to true,"changeSeconds" to 80,"timeMarginPct" to 15,"supportOverlapMm" to 1.5))["jobId"].asText()
        val worker=QueueWorker(queue,jobs,blobs,nodes,store,runner,launcher,app,props,json)
        try {
            worker.dispatch()
            val deadline=System.nanoTime()+TimeUnit.SECONDS.toNanos(15)
            while(jobs.findById(id).orElseThrow().status in activeStates && System.nanoTime()<deadline) Thread.sleep(50)
            val response=client.call("GET","/api/v2/jobs/$id")
            assertEquals("SUCCEEDED",response["status"].asText(),response.toString())
            // Фейковый пакер с окном пишет стол «12 из 30»; блок time доходит до ответа сервера.
            assertEquals(12,response["pack"]["objectCount"].asInt())
            assertEquals(12.0,response["pack"]["time"]["windowHours"].asDouble(),1e-9)
            assertEquals(30,response["pack"]["time"]["fullCount"].asInt())
            // --support-overlap-mm доходит до пакера; поля поддержек отчёта — до ответа.
            assertTrue(response["pack"]["supportsDetected"].asBoolean(),response.toString())
            assertEquals(1.5,response["pack"]["supportOverlapMm"].asDouble(),1e-9)
            assertEquals(1,response["artifacts"].size())
            assertTrue(response["artifacts"][0]["name"].asText().startsWith("cat_"))
            assertTrue(client.call("GET","/api/v2/jobs/$id?debug=true")["artifacts"].size()>1)
            client.call("DELETE","/api/v2/jobs/$id")
            client.call("GET","/api/v2/jobs/$id",expected=404)
        } finally {worker.stop()}
    }

    @Test fun `calibration job runs on the worker and publishes test files and values`() {
        val client=Client();client.guest()
        val file=client.upload()
        client.job(file,extra=mapOf("kind" to "CALIBRATION","variant" to "extreme"),expected=400)
        client.job(file,extra=mapOf("supportOverlapMm" to 21),expected=400)
        val id=client.job(file,extra=mapOf("kind" to "CALIBRATION","variant" to "perfectionist"))["jobId"].asText()
        val worker=QueueWorker(queue,jobs,blobs,nodes,store,runner,launcher,app,props,json)
        try {
            worker.dispatch()
            val deadline=System.nanoTime()+TimeUnit.SECONDS.toNanos(15)
            while(jobs.findById(id).orElseThrow().status in activeStates && System.nanoTime()<deadline) Thread.sleep(50)
            val response=client.call("GET","/api/v2/jobs/$id")
            assertEquals("SUCCEEDED",response["status"].asText(),response.toString())
            assertEquals("calibration",response["kind"].asText())
            val calibration=response["calibration"]
            assertEquals(30,calibration["changesExpected"].asInt())
            assertEquals(1500.0,calibration["estimateASeconds"].asDouble(),1e-9)
            assertEquals(2400.0,calibration["estimateBSeconds"].asDouble(),1e-9)
            assertEquals(20.5,calibration["slicerChangeSeconds"].asDouble(),1e-9)
            assertTrue(calibration["instructions"].asText().contains("calib_B.3mf"))
            assertTrue(response["pack"].isNull)
            // Файлы теста — под своими именами (на них ссылается инструкция); журнал — только в debug.
            assertEquals(setOf("calib_A.3mf","calib_B.3mf","instructions.md","calibration.json"),response["artifacts"].map { it["name"].asText() }.toSet())
        } finally {worker.stop()}
    }

    @Test fun `calibration is rejected for a printer without AMS changes`() {
        val client=Client();client.guest()
        val file=client.upload("Snapmaker U1","snapmaker_u1")
        val error=client.call("POST","/api/v2/jobs",mapOf("fileId" to file,"profileId" to "snapmaker_u1","kind" to "CALIBRATION"),400,headers=mapOf("Idempotency-Key" to newId()))
        assertTrue(error.toString().contains("для этого принтера калибровка не нужна"),error.toString())
    }

    @Test fun `restart requires confirmed stop increments fence and limits retry budget`() {
        val c=Client();c.guest();val id=c.job(c.upload())["jobId"].asText()
        queue.nodeRecoveryBegin("recover-node","boot-a","host-a")
        queue.nodePulse(WorkerEntity("recover-node","boot-a",2.0,4096L*1024*1024,1,0))
        val first=queue.claim("recover-node","boot-a")!!
        assertEquals(1,first.attemptCount)
        queue.runtimeLost(id,first.fence,"simulated connection loss")
        assertNull(queue.claim("recover-node","boot-a"))
        assertThrows(IllegalStateException::class.java) {queue.nodeRecoveryBegin("recover-node","boot-b","another-host")}
        queue.nodeRecoveryBegin("recover-node","boot-b","host-a")
        assertFalse(queue.nodePulse(WorkerEntity("recover-node","boot-a",2.0,4096L*1024*1024,1,0)))
        queue.nodeRecoveryConfirmed("recover-node","boot-b")
        assertEquals("RETRY_WAIT",jobs.findById(id).orElseThrow().status)
        assertFalse(queue.finish(id,first.fence,"{}",emptyList(),null))
        assertNull(queue.claim("recover-node","boot-a"))
        jdbc.update("update queue_jobs set available_at=now() where id=?",id)
        queue.nodePulse(WorkerEntity("recover-node","boot-b",2.0,4096L*1024*1024,1,0))
        val second=queue.claim("recover-node","boot-b")!!
        assertTrue(second.fence>first.fence)
        queue.retryStopped(id,second.fence,"second crash")
        assertEquals("FAILED",jobs.findById(id).orElseThrow().status)
        assertEquals(0L,jdbc.queryForObject("select reserved_seconds from compute_budgets where owner_id='GLOBAL'",Long::class.java))
        assertEquals(2,jdbc.queryForObject("select count(*) from job_attempts where job_id=?",Int::class.java,id))
    }
    @Test fun `compute reservations enforce global cap and cancellation refunds unused time`() {
        val old=props.globalComputeSeconds;props.globalComputeSeconds=props.jobTimeoutSeconds
        try {
            val a=Client();a.guest();val id=a.job(a.upload())["jobId"].asText()
            val b=Client();b.guest();val file=b.upload()
            b.call("POST","/api/v2/jobs",mapOf("fileId" to file,"profileId" to "bambu_p1s"),429,mapOf("Idempotency-Key" to newId()))
            a.call("DELETE","/api/v2/jobs/$id")
            assertNotNull(b.job(file)["jobId"])
        } finally {props.globalComputeSeconds=old}
    }
    @Test fun `publication refuses expired staging but retains successfully committed objects`() {
        val c=Client();c.guest();val id=c.job(c.upload())["jobId"].asText()
        nodes.save(WorkerEntity("publisher",newId(),2.0,4096L*1024*1024,1,0))
        val claim=queue.claim("publisher")!!
        val artifact=ArtifactEntity(jobId=id,name="packed.3mf",downloadName="cat_p1s_1_packed.3mf",objectKey="test/staged",sizeBytes=10)
        queue.stageObject(artifact.objectKey,10)
        jdbc.update("update object_gc set deleting=true where object_key=?",artifact.objectKey)
        assertThrows(IllegalStateException::class.java) {queue.finish(id,claim.fence,"{}",listOf(artifact),null)}
        jdbc.update("update object_gc set deleting=false where object_key=?",artifact.objectKey)
        assertTrue(queue.finish(id,claim.fence,"{}",listOf(artifact),null))
        assertFalse(queue.finish(id,claim.fence,"{}",listOf(artifact),null))
        assertEquals(0,jdbc.queryForObject("select count(*) from object_gc where object_key=?",Int::class.java,artifact.objectKey))
        assertEquals(1,jdbc.queryForObject("select count(*) from artifacts where object_key=?",Int::class.java,artifact.objectKey))
    }
    @Test fun `proof is owner bound one use and required before bytes are admitted`() {
        val old=props.requireChallenge;val difficulty=props.powDifficulty;props.requireChallenge=true;props.powDifficulty=8
        try {
            val a=Client();a.guest();val b=Client();b.guest()
            val ch=a.call("POST","/api/v2/challenges")
            var nonce=0L
            val hash=java.security.MessageDigest.getInstance("SHA-256")
            while(hash.digest("${ch["id"].asText()}:${ch["salt"].asText()}:$nonce".toByteArray())[0].toInt()!=0) nonce++
            val intent=mapOf("name" to "test.3mf","sizeBytes" to 100,"challengeId" to ch["id"].asText(),"nonce" to nonce.toString())
            a.call("POST","/api/v2/uploads",mapOf("name" to "test.3mf","sizeBytes" to 100),400)
            b.call("POST","/api/v2/uploads",intent,400)
            val upload=a.call("POST","/api/v2/uploads",intent)["fileId"].asText()
            a.call("DELETE","/api/v2/uploads/$upload")
            a.call("POST","/api/v2/uploads",intent,400)
        } finally {props.requireChallenge=old;props.powDifficulty=difficulty}
    }

    @Test fun `control plane rejects oversized json before allocating an object tree`() {
        val response=mvc.perform(post("/api/v2/auth/login").contentType("application/json").content("x".repeat(65537))).andReturn().response
        assertEquals(413,response.status)
    }

}
