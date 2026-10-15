package com.goatpacker.web.server

import com.goatpacker.web.service.Project3mf
import com.goatpacker.web.service.ProfileService
import jakarta.servlet.http.HttpServletRequest
import org.springframework.context.annotation.*
import org.springframework.jdbc.core.JdbcTemplate
import org.springframework.stereotype.Service
import org.springframework.transaction.annotation.Transactional
import org.springframework.web.bind.annotation.*
import software.amazon.awssdk.auth.credentials.*
import software.amazon.awssdk.core.sync.RequestBody as SdkBody
import software.amazon.awssdk.http.urlconnection.UrlConnectionHttpClient
import software.amazon.awssdk.regions.Region
import software.amazon.awssdk.services.s3.*
import software.amazon.awssdk.services.s3.model.*
import software.amazon.awssdk.services.s3.presigner.S3Presigner
import software.amazon.awssdk.services.s3.presigner.model.GetObjectPresignRequest
import java.net.URI
import java.nio.file.*
import java.security.MessageDigest
import java.time.*

interface ObjectStore {
    fun put(key:String,source:Path)
    fun get(key:String,target:Path)
    fun delete(key:String)
    fun signedGet(key:String,filename:String,seconds:Long):String?
}
class FileObjectStore(private val root:Path):ObjectStore {
    private fun path(key:String):Path {
        val p=root.resolve(key).normalize();require(p.startsWith(root.normalize()));return p
    }
    override fun put(key:String,source:Path) { val p=path(key);Files.createDirectories(p.parent);Files.copy(source,p,StandardCopyOption.REPLACE_EXISTING) }
    override fun get(key:String,target:Path) { Files.copy(path(key),target,StandardCopyOption.REPLACE_EXISTING) }
    override fun delete(key:String) { Files.deleteIfExists(path(key)) }
    override fun signedGet(key:String,filename:String,seconds:Long):String?=null
}
class S3Objects(props:PlatformProperties):ObjectStore {
    private val bucket=props.s3Bucket
    private val credentials=StaticCredentialsProvider.create(AwsBasicCredentials.create(props.s3AccessKey,props.s3SecretKey))
    private val config=S3Configuration.builder().pathStyleAccessEnabled(true).build()
    private val client=S3Client.builder().endpointOverride(URI(props.s3Endpoint)).region(Region.US_EAST_1)
        .requestChecksumCalculation(software.amazon.awssdk.core.checksums.RequestChecksumCalculation.WHEN_REQUIRED)
        .credentialsProvider(credentials).serviceConfiguration(config).httpClientBuilder(UrlConnectionHttpClient.builder())
        .overrideConfiguration { it.apiCallTimeout(Duration.ofSeconds(60)) }.build()
    private val signer=S3Presigner.builder().endpointOverride(URI(props.s3PublicEndpoint)).region(Region.US_EAST_1)
        .credentialsProvider(credentials).serviceConfiguration(config).build()
    override fun put(key:String,source:Path) {
        // Explicit standard Content-MD5 avoids aws-chunked checksum trailers across S3-compatible servers.
        val md5=MessageDigest.getInstance("MD5")
        Files.newInputStream(source).use { input -> val buffer=ByteArray(65536);while(true) {val n=input.read(buffer);if(n<0) break;md5.update(buffer,0,n)} }
        val checksum=java.util.Base64.getEncoder().encodeToString(md5.digest())
        client.putObject(PutObjectRequest.builder().bucket(bucket).key(key).contentMD5(checksum).build(),SdkBody.fromFile(source))
    }
    override fun get(key:String,target:Path) { Files.deleteIfExists(target);client.getObject(GetObjectRequest.builder().bucket(bucket).key(key).build(),target) }
    override fun delete(key:String) { client.deleteObject(DeleteObjectRequest.builder().bucket(bucket).key(key).build()) }
    override fun signedGet(key:String,filename:String,seconds:Long):String {
        val disposition=org.springframework.http.ContentDisposition.attachment().filename(filename,Charsets.UTF_8).build().toString()
        return signer.presignGetObject(GetObjectPresignRequest.builder().signatureDuration(Duration.ofSeconds(seconds))
            .getObjectRequest(GetObjectRequest.builder().bucket(bucket).key(key).responseContentDisposition(disposition).responseContentType("application/octet-stream").build()).build()).url().toString()
    }
}
@Configuration @Profile("server")
class StorageConfiguration {
    @Bean fun objectStore(props:PlatformProperties):ObjectStore = if(props.storageMode=="file") FileObjectStore(Path.of(props.localStorageRoot).toAbsolutePath()) else S3Objects(props)
}
fun safeName(raw:String):String = raw.substringAfterLast('/').substringAfterLast('\\')
    .replace(Regex("[^\\p{L}\\p{N}._ -]"),"_").trim().take(180).ifBlank { "model.3mf" }

@Service @Profile("server")
class Quotas(private val jdbc:JdbcTemplate,private val props:PlatformProperties) {
    fun ownerBytes(owner:String):Long = jdbc.queryForObject("""select
        coalesce((select sum(expected_bytes) from uploads where owner_id=?),0)+
        coalesce((select sum(reserved_bytes) from queue_jobs where owner_id=?),0)+
        coalesce((select sum(a.size_bytes) from artifacts a join queue_jobs j on j.id=a.job_id where j.owner_id=?),0)""",Long::class.java,owner,owner,owner)!!
    fun totalBytes():Long = jdbc.queryForObject("""select coalesce((select sum(size_bytes) from blobs),0)+
        coalesce((select sum(size_bytes) from artifacts),0)+coalesce((select sum(expected_bytes) from uploads where state in ('UPLOADING','RECEIVING')),0)+
        coalesce((select sum(reserved_bytes) from queue_jobs),0)+coalesce((select sum(size_bytes) from object_gc),0)""",Long::class.java)!!
    fun reserve(owner:PrincipalEntity,bytes:Long) {
        if(owner.blocked) throw ServerError(403,"BLOCKED","Доступ заблокирован")
        if(bytes<0 || bytes>owner.storageQuota-ownerBytes(owner.id)) limited("Квота хранения исчерпана")
        if(bytes>props.storageLimitBytes-totalBytes()) limited("Хранилище временно заполнено")
    }
}
@Service @Profile("server")
class UploadTransactions(private val uploads:UploadRepository,private val blobs:BlobRepository,
    private val owners:PrincipalRepository,private val platform:PlatformRepository,private val quotas:Quotas,
    private val props:PlatformProperties,private val jdbc:JdbcTemplate,private val challenges:Challenges) {
    @Transactional fun create(owner:String,name:String,size:Long,challengeId:String?=null,nonce:String?=null):UploadEntity {
        if(!name.endsWith(".3mf",true)) bad("Нужен файл .3mf")
        if(size !in 1..props.maxUploadBytes) throw ServerError(413,"FILE_TOO_LARGE","Допустимо до ${props.maxUploadBytes/1024/1024} МиБ")
        platform.locked();val principal=owners.locked(owner) ?: missing()
        if(jdbc.queryForObject("select maintenance from operational_state where id=1",Boolean::class.java)==true) throw ServerError(503,"MAINTENANCE","Сервис временно на обслуживании")
        challenges.consume(owner,challengeId,nonce);quotas.reserve(principal,size)
        if(jdbc.queryForObject("select count(*) from uploads where state in ('UPLOADING','RECEIVING')",Long::class.java)!!>=2) limited("Все места загрузки заняты")
        if(uploads.findAllByOwnerId(owner).any { it.state in setOf("UPLOADING","RECEIVING") }) limited("У вас уже есть активная загрузка")
        return uploads.save(UploadEntity(ownerId=owner,originalName=safeName(name),expectedBytes=size))
    }
    @Transactional fun begin(id:String,owner:String,length:Long):UploadEntity {
        platform.locked();val upload=owned(id,owner)
        if(upload.state!="UPLOADING" || upload.expiresAt?.isBefore(Instant.now())==true) bad("Загрузка истекла или уже началась")
        if(length!=upload.expectedBytes) bad("Размер загрузки не совпадает с зарезервированным")
        upload.state="RECEIVING"
        jdbc.update("insert into object_gc(object_key,size_bytes,not_before) values (?,?,now()+interval '2 hours') on conflict do nothing","inputs/$id",upload.expectedBytes)
        return upload
    }
    @Transactional fun finish(id:String,owner:String,hash:String,key:String,inspection:Project3mf.Inspection,profileId:String?):Pair<UploadEntity,Boolean> {
        platform.locked();val p=owners.locked(owner) ?: missing();if(p.blocked) throw ServerError(403,"BLOCKED","Доступ заблокирован")
        val upload=owned(id,owner);if(upload.state!="RECEIVING" || upload.expiresAt?.isBefore(Instant.now())==true) bad("Загрузка больше не активна")
        check(jdbc.queryForList("select object_key from object_gc where object_key=? and not deleting and not_before>now() for update",key).isNotEmpty()) {"Upload staging expired"}
        val existing=blobs.findBySha256AndSizeBytes(hash,upload.expectedBytes)
        val blob=existing ?: blobs.save(BlobEntity(sha256=hash,sizeBytes=upload.expectedBytes,objectKey=key))
        if(existing==null) jdbc.update("delete from object_gc where object_key=?",key)
        upload.blobId=blob.id;upload.state="READY";upload.printerModel=inspection.printerModel;upload.profileId=profileId
        upload.expiresAt=if(p.kind=="GUEST") Instant.now().plusSeconds(86400) else null
        return upload to (existing!=null)
    }
    @Transactional fun abort(id:String,owner:String) { platform.locked();uploads.findById(id).orElse(null)?.takeIf { it.ownerId==owner && it.state!="READY" }?.let(uploads::delete) }
    @Transactional fun delete(id:String,owner:String) { platform.locked();val file=owned(id,owner);if(file.state=="RECEIVING") bad("Дождитесь конца загрузки");uploads.delete(file) }
    fun list(owner:String)=uploads.findAllByOwnerId(owner).sortedByDescending { it.createdAt }.take(100)
    fun owned(id:String,owner:String):UploadEntity=uploads.findById(id).orElse(null)?.takeIf { it.ownerId==owner } ?: missing()
}
@RestController @RequestMapping("/api/v2/uploads") @Profile("server")
class UploadController(private val tx:UploadTransactions,private val store:ObjectStore,private val inspector:Project3mf,
    private val profiles:ProfileService,private val limits:AdmissionLimits,private val props:PlatformProperties) {
    data class Intent(val name:String="",val sizeBytes:Long=0,val challengeId:String?=null,val nonce:String?=null)
    @PostMapping fun create(@RequestBody input:Intent,request:HttpServletRequest):Map<String,Any?> {
        val user=actor();limits.hit("uploads:${request.remoteAddr}",10,3600);limits.hit("upload-global",100,3600)
        return view(tx.create(user.id,input.name,input.sizeBytes,input.challengeId,input.nonce))
    }
    @PutMapping("/{id}/content") fun content(@PathVariable id:String,request:HttpServletRequest):Map<String,Any?> {
        val owner=actor().id;val upload=tx.begin(id,owner,request.contentLengthLong)
        val path=Files.createTempFile("goat-upload-",".3mf");val key="inputs/$id"
        try {
            val deadline=System.nanoTime()+java.util.concurrent.TimeUnit.SECONDS.toNanos(props.uploadTimeoutSeconds)
            val digest=MessageDigest.getInstance("SHA-256");var copied=0L
            request.inputStream.use { input->Files.newOutputStream(path).use { output->
                val buffer=ByteArray(64*1024)
                while(true) { if(System.nanoTime()>deadline) bad("Время загрузки превышено");val n=input.read(buffer);if(n<0) break;copied+=n;if(copied>upload.expectedBytes) bad("Размер файла превышен");digest.update(buffer,0,n);output.write(buffer,0,n) }
            } }
            if(copied!=upload.expectedBytes) bad("Загрузка неполная")
            val inspection=inspector.inspect(path);inspector.requireModel(inspection)
            val detection=profiles.detect(inspection.printerModel)
            store.put(key,path)
            val (ready,_)=tx.finish(id,owner,digest.digest().joinToString(""){"%02x".format(it)},key,inspection,detection.profileId)
            return view(ready)
        } catch(ex:Exception) {
            // Only durable GC may delete: finish may have committed despite a lost DB reply.
            runCatching { tx.abort(id,owner) };throw ex
        } finally { Files.deleteIfExists(path) }
    }
    @GetMapping fun list()=tx.list(actor().id).map(::view)
    @DeleteMapping("/{id}") fun delete(@PathVariable id:String) {tx.delete(id,actor().id)}
    @GetMapping("/{id}") fun get(@PathVariable id:String)=view(tx.owned(id,actor().id))
    private fun view(u:UploadEntity)=mapOf("fileId" to u.id,"originalName" to u.originalName,"sizeBytes" to u.expectedBytes,"state" to u.state,"profileId" to u.profileId,"printerModel" to u.printerModel,"expiresAt" to u.expiresAt)
}
