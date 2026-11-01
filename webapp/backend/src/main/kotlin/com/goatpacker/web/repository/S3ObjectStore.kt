package com.goatpacker.web.repository

import com.goatpacker.web.config.PlatformProperties
import com.goatpacker.web.util.digest
import org.springframework.http.ContentDisposition
import software.amazon.awssdk.auth.credentials.AwsBasicCredentials
import software.amazon.awssdk.auth.credentials.StaticCredentialsProvider
import software.amazon.awssdk.core.checksums.RequestChecksumCalculation
import software.amazon.awssdk.core.sync.RequestBody
import software.amazon.awssdk.http.urlconnection.UrlConnectionHttpClient
import software.amazon.awssdk.regions.Region
import software.amazon.awssdk.services.s3.S3Client
import software.amazon.awssdk.services.s3.S3Configuration
import software.amazon.awssdk.services.s3.model.DeleteObjectRequest
import software.amazon.awssdk.services.s3.model.GetObjectRequest
import software.amazon.awssdk.services.s3.model.PutObjectRequest
import software.amazon.awssdk.services.s3.presigner.S3Presigner
import software.amazon.awssdk.services.s3.presigner.model.GetObjectPresignRequest
import java.net.URI
import java.nio.file.Files
import java.nio.file.Path
import java.time.Duration
import java.util.Base64

/** S3-совместимое хранилище (SeaweedFS): приватные объекты, скачивание по подписанным ссылкам. */
class S3ObjectStore(props: PlatformProperties) : ObjectStore {
    private val bucket = props.s3Bucket
    private val credentials = StaticCredentialsProvider.create(AwsBasicCredentials.create(props.s3AccessKey, props.s3SecretKey))
    private val config = S3Configuration.builder().pathStyleAccessEnabled(true).build()
    private val client = S3Client.builder()
        .endpointOverride(URI(props.s3Endpoint))
        .region(REGION)
        .requestChecksumCalculation(RequestChecksumCalculation.WHEN_REQUIRED)
        .credentialsProvider(credentials)
        .serviceConfiguration(config)
        .httpClientBuilder(UrlConnectionHttpClient.builder())
        .overrideConfiguration { it.apiCallTimeout(API_CALL_TIMEOUT) }
        .build()
    private val signer = S3Presigner.builder()
        .endpointOverride(URI(props.s3PublicEndpoint))
        .region(REGION)
        .credentialsProvider(credentials)
        .serviceConfiguration(config)
        .build()

    override fun put(key: String, source: Path) {
        // Explicit standard Content-MD5 avoids aws-chunked checksum trailers across S3-compatible servers.
        val request = PutObjectRequest.builder().bucket(bucket).key(key).contentMD5(md5Base64(source)).build()
        client.putObject(request, RequestBody.fromFile(source))
    }

    override fun get(key: String, target: Path) {
        Files.deleteIfExists(target)
        client.getObject(GetObjectRequest.builder().bucket(bucket).key(key).build(), target)
    }

    override fun delete(key: String) {
        client.deleteObject(DeleteObjectRequest.builder().bucket(bucket).key(key).build())
    }

    override fun signedGet(key: String, filename: String, seconds: Long): String {
        val disposition = ContentDisposition.attachment().filename(filename, Charsets.UTF_8).build().toString()
        val getObject = GetObjectRequest.builder()
            .bucket(bucket)
            .key(key)
            .responseContentDisposition(disposition)
            .responseContentType("application/octet-stream")
            .build()
        val presign = GetObjectPresignRequest.builder()
            .signatureDuration(Duration.ofSeconds(seconds))
            .getObjectRequest(getObject)
            .build()
        return signer.presignGetObject(presign).url().toString()
    }

    private fun md5Base64(source: Path): String = Base64.getEncoder().encodeToString(source.digest("MD5"))

    private companion object {
        /** SeaweedFS регион не проверяет, но подпись требует любого. */
        val REGION: Region = Region.US_EAST_1
        val API_CALL_TIMEOUT: Duration = Duration.ofSeconds(60)
    }
}
