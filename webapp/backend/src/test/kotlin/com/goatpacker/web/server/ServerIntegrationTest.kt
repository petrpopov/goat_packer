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
        fun bytes(path:String,expected:Int=200):ByteArray {
            val builder=request(HttpMethod.GET,path).with { it.remoteAddr=ip;it }
            if(cookies.isNotEmpty()) builder.cookie(*cookies.values.toTypedArray())
            val res=mvc.perform(builder).andReturn().response
            assertEquals(expected,res.status,"GET $path");return res.contentAsByteArray
        }
        fun guest()=call("POST","/api/v2/auth/guest")
        fun login(name:String="admin",pass:String="Test-admin-secret-123",expected:Int=200)=call("POST","/api/v2/auth/login",mapOf("login" to name,"password" to pass),expected)
        fun upload(printer:String="Bambu Lab P1S",profile:String="bambu_p1s"):String {
            val data=descriptorProject3mf(printer)
            val id=call("POST","/api/v2/uploads",mapOf("name" to "cat.3mf","sizeBytes" to data.size))["fileId"].asText()
            assertEquals(profile,call("PUT","/api/v2/uploads/$id/content",data,headers=mapOf("Origin" to "http://localhost:3007"))["profileId"].asText())
            return id
        }
        fun uploadRaw(name:String,data:ByteArray):com.fasterxml.jackson.databind.JsonNode {
            val id=call("POST","/api/v2/uploads",mapOf("name" to name,"sizeBytes" to data.size))["fileId"].asText()
            return call("PUT","/api/v2/uploads/$id/content",data,headers=mapOf("Origin" to "http://localhost:3007"))
        }
        fun job(file:String,key:String=newId(),extra:Map<String,Any> = emptyMap(),expected:Int=200)=call("POST","/api/v2/jobs",mapOf("fileId" to file,"profileId" to "bambu_p1s")+extra,expected,headers=mapOf("Idempotency-Key" to key))
    }
    @BeforeEach fun clearQueue() {
        jdbc.update("delete from compute_budgets");jdbc.update("delete from bot_challenges");jdbc.update("delete from object_gc");jdbc.update("delete from worker_nodes");jdbc.update("delete from artifacts");jdbc.update("delete from queue_jobs");jdbc.update("delete from uploads");jdbc.update("delete from blobs")
        jdbc.update("update platform_config set max_parallel=1,paused=false where id=1")
    }
    @Test fun `admission limits switched off let a guest run past the daily and active job limits`() {
        props.admissionLimits=false
        try {
            val client=Client();client.guest()
            val file=client.upload()
            // Гостю обычно 4 задачи в сутки и 2 активных: без лимитов проходят все шесть.
            repeat(6) { client.job(file) }
        } finally {props.admissionLimits=true}
        val other=Client();other.guest()
        val file=other.upload()
        repeat(2) { other.job(file) }
        other.job(file,expected=429)
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
    @Test fun `stl packs on any catalog printer or a custom bed, converted by source_project before the packer, and schedules from the packed project`() {
        val c=Client();c.guest()
        assertEquals("Нужен файл .3mf или .stl",c.call("POST","/api/v2/uploads",mapOf("name" to "cat.obj","sizeBytes" to 10),expected=400)["message"].asText())
        val broken=c.call("POST","/api/v2/uploads",mapOf("name" to "broken.stl","sizeBytes" to 10))["fileId"].asText()
        assertEquals(com.goatpacker.web.service.StlInputs.INVALID_STL,c.call("PUT","/api/v2/uploads/$broken/content","PK34567890".toByteArray(),400,headers=mapOf("Origin" to "http://localhost:3007"))["message"].asText())
        val stl=c.uploadRaw("cat.stl","solid cat\nfacet normal 0 0 1\nendsolid cat\n".toByteArray())
        assertEquals("READY",stl["state"].asText());assertEquals("stl",stl["kind"].asText());assertTrue(stl["profileId"].isNull,stl.toString())
        val file=stl["fileId"].asText()
        fun send(body:Map<String,Any?>,expected:Int)=c.call("POST","/api/v2/jobs",body,expected,headers=mapOf("Idempotency-Key" to newId()))
        assertEquals("STL можно только упаковать: для расписания и калибровки нужен проект .3mf",send(mapOf("fileId" to file,"profileId" to "bambu_p1s","kind" to "CALIBRATION"),400)["message"].asText())
        // Локальный раннер сервера без GOAT_SLICER_PRESETS: отказ при постановке, а не в воркере.
        assertEquals(com.goatpacker.web.service.StlInputs.LOCAL_PRESETS_MISSING,send(mapOf("fileId" to file,"profileId" to "prusa_mk4s"),400)["message"].asText())
        val presets=Files.createDirectories(root.resolve("orca-profiles"))
        val saved=listOf(app.slicerPresets,app.sourceProjectBin,app.orcaSlicer)
        app.slicerPresets=presets.toString();app.orcaSlicer="/fake/orca"
        app.sourceProjectBin=com.goatpacker.web.FakeBins.install(root,"fake-source-project.sh");app.resolveAbsolutePaths()
        val worker=QueueWorker(queue,jobs,blobs,nodes,store,runner,launcher,app,props,json)
        try {
            val catalog=send(mapOf("fileId" to file,"profileId" to "prusa_mk4s"),200)["jobId"].asText()
            val custom=send(mapOf("fileId" to file,"customBed" to mapOf("width" to 220,"depth" to 220,"height" to 250)),200)["jobId"].asText()
            assertEquals("Custom Printer",json.readTree(jobs.findById(custom).orElseThrow().profileJson)["printer_name"].asText())
            for(id in listOf(catalog,custom)) {
                worker.dispatch()
                val deadline=System.nanoTime()+TimeUnit.SECONDS.toNanos(15)
                while(jobs.findById(id).orElseThrow().status in activeStates && System.nanoTime()<deadline) Thread.sleep(50)
                val response=c.call("GET","/api/v2/jobs/$id")
                assertEquals("SUCCEEDED",response["status"].asText(),response.toString())
                assertTrue(response["artifacts"][0]["name"].asText().startsWith("cat_"),response.toString())
            }
            // «В расписание» после упаковки STL: источник — packed.3mf, т. е. проект. Имя становится .3mf, и
            // воркер кладёт его как input.3mf (раньше — input.stl, и daily_scheduler не находил вход).
            // Профиль каталога несёт слайсер образа: локальный раннер подменяет его (иначе фейк отказывает).
            val schedule=send(mapOf("kind" to "SCHEDULE","packJobId" to catalog,"profileId" to "prusa_mk4s"),200)["jobId"].asText()
            assertEquals("cat.3mf",jobs.findById(schedule).orElseThrow().originalName)
            worker.dispatch()
            val deadline=System.nanoTime()+TimeUnit.SECONDS.toNanos(15)
            while(jobs.findById(schedule).orElseThrow().status in activeStates && System.nanoTime()<deadline) Thread.sleep(50)
            val response=c.call("GET","/api/v2/jobs/$schedule")
            assertEquals("SUCCEEDED",response["status"].asText(),response.toString())
            assertTrue(response["artifacts"].any { it["name"].asText().startsWith("cat_prusa_mk4s_") && it["name"].asText().endsWith("_schedule.3mf") },response.toString())
        } finally {
            worker.stop()
            app.slicerPresets=saved[0];app.sourceProjectBin=saved[1];app.orcaSlicer=saved[2];app.resolveAbsolutePaths()
        }
    }
    @Test fun `unknown printer upload carries the project bed for the custom-bed form`() {
        val c=Client();c.guest()
        val settings=mapOf("printable_area" to listOf("0x0","250x0","250x210","0x210"),"bed_exclude_area" to listOf("0x0","20x0","20x30","0x30"),"printable_height" to "220")
        val unknown=c.uploadRaw("cat.3mf",descriptorProject3mf("Acme Unknown 3000",settings))
        assertTrue(unknown["profileId"].isNull);assertEquals("3mf",unknown["kind"].asText())
        assertEquals(listOf(0.0,0.0,250.0,210.0),unknown["projectBed"]["printableArea"].map { it.asDouble() })
        assertEquals(listOf(listOf(0.0,0.0,20.0,30.0)),unknown["projectBed"]["exclusionRectangles"].map { r->r.map { it.asDouble() } })
        assertEquals(unknown["projectBed"],c.call("GET","/api/v2/uploads/${unknown["fileId"].asText()}")["projectBed"])
        assertTrue(c.uploadRaw("p1s.3mf",descriptorProject3mf("Bambu Lab P1S",settings))["projectBed"].isNull)
    }
    @Test fun `pack status carries originalName, profile and params for the read-only workspace, and the input downloads like an artifact`() {
        val a=Client();a.guest();val b=Client();b.guest()
        val data=descriptorProject3mf("Bambu Lab P1S")
        val file=a.upload()
        val created=a.job(file,extra=mapOf("clearance" to 1.5,"finalSlice" to false));val id=created["jobId"].asText()
        val st=a.call("GET","/api/v2/jobs/$id")
        assertEquals("cat.3mf",st["originalName"].asText())
        assertEquals("bambu_p1s",st["profile"]["id"].asText())
        assertEquals(profilesOf(a).first { it["id"].asText()=="bambu_p1s" },st["profile"])
        assertEquals(1.5,st["params"]["clearance"].asDouble(),1e-9);assertFalse(st["params"]["finalSlice"].asBoolean())
        assertEquals("search",st["params"]["layout"].asText());assertTrue(st["params"]["customBed"].isNull)
        // Список лёгкий: профиль и настройки только в статусе одной задачи.
        assertFalse(a.call("GET","/api/v2/jobs")["items"][0].has("profile"))
        val dl=a.call("POST","/api/v2/jobs/$id/input/download")
        assertEquals("cat.3mf",dl["filename"].asText())
        assertArrayEquals(data,a.bytes(dl["url"].asText()))
        // Без доступа — как для артефактов; по токену чтения — можно.
        b.call("POST","/api/v2/jobs/$id/input/download",expected=404)
        b.bytes("/api/v2/jobs/$id/input/file",404)
        b.call("POST","/api/v2/jobs/$id/input/download",headers=mapOf("X-Job-Token" to created["readToken"].asText()))
        // Не упаковка: вход не отдаётся, профиль и настройки в статусе null.
        val sid=a.job(file)["jobId"].asText();jdbc.update("update queue_jobs set kind='SCHEDULE' where id=?",sid)
        a.call("POST","/api/v2/jobs/$sid/input/download",expected=400)
        assertTrue(a.call("GET","/api/v2/jobs/$sid")["params"].isNull)
    }
    private fun profilesOf(c:Client)=c.call("GET","/api/v2/profiles")
    @Test fun `custom bed job packs an unknown printer 3MF with an ad-hoc profile from the confirmed project bed`() {
        val c=Client();c.guest()
        val settings=mapOf("printable_area" to listOf("-5x0","245x0","245x210","-5x210"),"bed_exclude_area" to listOf("0x0"),"printable_height" to "220")
        val file=c.uploadRaw("cat.3mf",descriptorProject3mf("Acme Unknown 3000",settings))["fileId"].asText()
        val bed=mapOf("width" to 250,"depth" to 210,"height" to 220)
        fun send(body:Map<String,Any?>,expected:Int)=c.call("POST","/api/v2/jobs",body,expected,headers=mapOf("Idempotency-Key" to newId()))
        send(mapOf("fileId" to file,"profileId" to "bambu_p1s","customBed" to bed),400)
        send(mapOf("fileId" to file),400)
        send(mapOf("fileId" to file,"kind" to "SCHEDULE","customBed" to bed),400)
        send(mapOf("fileId" to file,"customBed" to bed+("height" to 1001)),400)
        send(mapOf("fileId" to file,"customBed" to bed+("margin" to 31)),400)
        // Слайсер режет 3MF по столу проекта: размеры менять нельзя, только отступ.
        assertEquals(com.goatpacker.web.service.CustomProfile.BED_FROM_PROJECT,send(mapOf("fileId" to file,"customBed" to bed+("width" to 300)),400)["message"].asText())
        // Распознанный принтер — только своим профилем каталога.
        val p1s=c.uploadRaw("p1s.3mf",descriptorProject3mf("Bambu Lab P1S"))["fileId"].asText()
        assertEquals(com.goatpacker.web.service.CustomProfile.RECOGNISED_PRINTER,send(mapOf("fileId" to p1s,"customBed" to mapOf("width" to 256,"depth" to 256,"height" to 250)),400)["message"].asText())
        val id=send(mapOf("fileId" to file,"customBed" to bed+("margin" to 7)),200)["jobId"].asText()
        val record=jobs.findById(id).orElseThrow()
        assertEquals("custom",record.profileId)
        val profile=json.readTree(record.profileJson)
        // Стол — проекта: его начало координат и исключения; отступ — введённый.
        assertEquals(listOf(-5.0,0.0,245.0,210.0),profile["printable_area"].map { it.asDouble() })
        assertEquals(7.0,profile["edge_margin_mm"].asDouble(),1e-9)
        val st=c.call("GET","/api/v2/jobs/$id")
        assertEquals("custom",st["profile"]["id"].asText());assertEquals(250.0,st["profile"]["bedWidthMm"].asDouble(),1e-9)
        assertEquals(210.0,st["profile"]["bedDepthMm"].asDouble(),1e-9);assertEquals(7.0,st["profile"]["edgeMarginMm"].asDouble(),1e-9)
        assertEquals(7.0,st["params"]["customBed"]["margin"].asDouble(),1e-9);assertEquals(250.0,st["params"]["customBed"]["width"].asDouble(),1e-9)
        assertEquals("Acme Unknown 3000",profile["printer_name"].asText());assertEquals("orca",profile["slicer_kind"].asText())
        val worker=QueueWorker(queue,jobs,blobs,nodes,store,runner,launcher,app,props,json)
        try {
            worker.dispatch()
            val deadline=System.nanoTime()+TimeUnit.SECONDS.toNanos(15)
            while(jobs.findById(id).orElseThrow().status in activeStates && System.nanoTime()<deadline) Thread.sleep(50)
            val response=c.call("GET","/api/v2/jobs/$id")
            assertEquals("SUCCEEDED",response["status"].asText(),response.toString())
            assertTrue(response["artifacts"][0]["name"].asText().contains("_custom_"),response.toString())
        } finally {worker.stop()}
    }
    @Test fun `unknown printer 3MF needs a custom bed, the catalog pick is refused`() {
        val c=Client();c.guest()
        val file=c.uploadRaw("cat.3mf",descriptorProject3mf("Acme Unknown 3000"))["fileId"].asText()
        assertEquals("Принтер файла не соответствует выбранному профилю",c.job(file,expected=400)["message"].asText())
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
            // layout-событие пакера (local runner ставит GOAT_LOG_FORMAT=json) живёт и после успеха.
            assertEquals(2,response["layout"]["placements"].size(),response.toString())
            // Список не тащит раскладку (до десятков КБ на строку при опросе раз в 5 с); одиночный статус — тащит.
            val listed=client.call("GET","/api/v2/jobs").get("items").first { it["jobId"].asText()==id }
            assertTrue(listed["layout"]==null || listed["layout"].isNull,listed.toString())
            assertEquals(1,response["artifacts"].size())
            assertTrue(response["artifacts"][0]["name"].asText().startsWith("cat_"))
            assertTrue(client.call("GET","/api/v2/jobs/$id?debug=true")["artifacts"].size()>1)
            client.call("DELETE","/api/v2/jobs/$id")
            client.call("GET","/api/v2/jobs/$id",expected=404)
        } finally {worker.stop()}
    }

    @Test fun `pack without the final slice keeps its state and the finish job slices it`() {
        val client=Client();client.guest()
        val file=client.upload()
        // Блоб состояния ставит только сервер.
        client.job(file,extra=mapOf("finishStateBlobId" to "x"),expected=400)
        val pack=client.job(file,extra=mapOf("finalSlice" to false))["jobId"].asText()
        val worker=QueueWorker(queue,jobs,blobs,nodes,store,runner,launcher,app,props,json)
        fun await(id:String):com.fasterxml.jackson.databind.JsonNode {
            worker.dispatch()
            val deadline=System.nanoTime()+TimeUnit.SECONDS.toNanos(15)
            while(jobs.findById(id).orElseThrow().status in activeStates && System.nanoTime()<deadline) Thread.sleep(50)
            return client.call("GET","/api/v2/jobs/$id?debug=true")
        }
        try {
            val packed=await(pack)
            assertEquals("SUCCEEDED",packed["status"].asText(),packed.toString())
            assertEquals(false,packed["pack"]["finalSliced"].asBoolean(),packed.toString())
            assertTrue(packed["artifacts"].any { it["name"].asText()=="pack_state.json" },packed.toString())
            // Проверка нарезкой — внутри той же задачи: ответ с id родителя, ребёнок скрыт; фейковый пакер падает, если состояния нет рядом.
            val key=newId()
            val answered=client.call("POST","/api/v2/jobs/$pack/check",headers=mapOf("Idempotency-Key" to key))["jobId"].asText()
            assertEquals(pack,answered)
            // Повтор того же ключа не создаёт второго ребёнка.
            assertEquals(pack,client.call("POST","/api/v2/jobs/$pack/check",headers=mapOf("Idempotency-Key" to key))["jobId"].asText())
            val child=jobs.findById(pack).orElseThrow().checkJobId!!
            assertEquals(pack,jobs.findById(child).orElseThrow().parentJobId)
            assertEquals(1,jdbc.queryForObject("select count(*) from queue_jobs where parent_job_id=?",Int::class.java,pack))
            // Пока проверка не кончилась: в статусе родителя она в очереди/работе, второй запуск отклоняется.
            assertTrue(client.call("GET","/api/v2/jobs/$pack")["check"]["status"].asText() in activeStates)
            client.call("POST","/api/v2/jobs/$pack/check",expected=400,headers=mapOf("Idempotency-Key" to newId()))
            worker.dispatch()
            val deadline=System.nanoTime()+TimeUnit.SECONDS.toNanos(15)
            while(jobs.findById(child).orElseThrow().status in activeStates && System.nanoTime()<deadline) Thread.sleep(50)
            val finished=client.call("GET","/api/v2/jobs/$pack?debug=true")
            val check=finished["check"]
            assertEquals("SUCCEEDED",check["status"].asText(),finished.toString())
            assertEquals(true,check["pack"]["finalSliced"].asBoolean(),finished.toString())
            assertFalse(finished.toString().contains(child),"id ребёнка не должен попадать в ответ")
            // Родитель остаётся собой; артефакты проверки скачиваются по id родителя.
            assertEquals(false,finished["pack"]["finalSliced"].asBoolean())
            assertTrue(check["artifacts"].all { it["downloadUrl"].asText().startsWith("/api/v2/jobs/$pack/artifacts/") },check.toString())
            assertTrue(check["artifacts"].none { it["name"].asText()=="pack_state.json" },check.toString())
            val childArtifact=check["artifacts"].first { it["name"].asText().endsWith(".3mf") }
            assertTrue(client.call("POST",childArtifact["downloadUrl"].asText(),emptyMap<String,String>())["url"].asText().isNotBlank())
            assertTrue(client.bytes("/api/v2/jobs/$pack/artifacts/${childArtifact["id"].asText()}/file").isNotEmpty())
            // Чужой задачей (другой упаковкой того же пользователя) этот артефакт не отдаётся.
            val other=client.job(file,extra=mapOf("finalSlice" to false))["jobId"].asText()
            client.call("POST","/api/v2/jobs/$other/artifacts/${childArtifact["id"].asText()}/download",emptyMap<String,String>(),404)
            client.bytes("/api/v2/jobs/$other/artifacts/${childArtifact["id"].asText()}/file",404)
            // Ребёнок скрыт: в списке одна строка на упаковку, по его id статус недоступен.
            val listed=client.call("GET","/api/v2/jobs")
            assertEquals(2,listed["total"].asInt(),listed.toString())
            assertTrue(listed["items"].none { it["jobId"].asText()==child })
            assertTrue(listed["items"].first { it["jobId"].asText()==pack }["check"]["status"].asText()=="SUCCEEDED")
            client.call("GET","/api/v2/jobs/$child",expected=404)
            // Суточный лимит гостя (4) проверка не тратит: посчитана упаковка (+ «other»).
            val owner=jobs.findById(pack).orElseThrow().ownerId
            assertEquals(2,jdbc.queryForObject("select used from request_limits where key=?",Int::class.java,"jobs:$owner"))
            // Стол уже проверен нарезкой; через старый путь packJobId — то же самое.
            client.call("POST","/api/v2/jobs/$pack/check",expected=400,headers=mapOf("Idempotency-Key" to newId()))
            client.call("POST","/api/v2/jobs",mapOf("kind" to "PACK","packJobId" to pack),400,headers=mapOf("Idempotency-Key" to newId()))
            // Удаление упаковки уносит ребёнка, а его артефакты уходят в GC.
            val keys=jdbc.queryForList("select object_key from artifacts where job_id=?",String::class.java,child)
            assertTrue(keys.isNotEmpty())
            client.call("DELETE","/api/v2/jobs/$pack")
            assertEquals(0,jdbc.queryForObject("select count(*) from queue_jobs where id=?",Int::class.java,child))
            keys.forEach { k->assertEquals(1,jdbc.queryForObject("select count(*) from object_gc where object_key=?",Int::class.java,k),k) }
        } finally {worker.stop()}
    }

    @Test fun `a failed check may be re-run and old clients still get the parent id`() {
        val client=Client();client.guest()
        val file=client.upload()
        val pack=client.job(file,extra=mapOf("finalSlice" to false))["jobId"].asText()
        val worker=QueueWorker(queue,jobs,blobs,nodes,store,runner,launcher,app,props,json)
        try {
            worker.dispatch()
            val deadline=System.nanoTime()+TimeUnit.SECONDS.toNanos(15)
            while(jobs.findById(pack).orElseThrow().status in activeStates && System.nanoTime()<deadline) Thread.sleep(50)
            // Чужая задача проверять не может.
            val stranger=Client();stranger.guest()
            stranger.call("POST","/api/v2/jobs/$pack/check",expected=404,headers=mapOf("Idempotency-Key" to newId()))
            // Старый путь: POST /jobs с packJobId отвечает id родителя.
            assertEquals(pack,client.call("POST","/api/v2/jobs",mapOf("kind" to "PACK","packJobId" to pack),headers=mapOf("Idempotency-Key" to newId()))["jobId"].asText())
            val first=jobs.findById(pack).orElseThrow().checkJobId!!
            // Отмена очереди: проверка CANCELLED, её можно запустить снова — ссылка заменяется.
            client.call("POST","/api/v2/jobs/$pack/check/cancel")
            assertEquals("CANCELLED",client.call("GET","/api/v2/jobs/$pack")["check"]["status"].asText())
            client.call("POST","/api/v2/jobs/$pack/check",headers=mapOf("Idempotency-Key" to newId()))
            val second=jobs.findById(pack).orElseThrow().checkJobId!!
            assertNotEquals(first,second)
            assertEquals("QUEUED",client.call("GET","/api/v2/jobs/$pack")["check"]["status"].asText())
            // Ожидающий в очереди ребёнок отменяется и уходит каскадом вместе с упаковкой.
            client.call("DELETE","/api/v2/jobs/$pack")
            assertEquals(0,jdbc.queryForObject("select count(*) from queue_jobs where id in (?,?,?)",Int::class.java,pack,first,second))
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
