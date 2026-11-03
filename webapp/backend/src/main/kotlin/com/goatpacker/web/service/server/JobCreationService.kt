package com.goatpacker.web.service.server

import com.fasterxml.jackson.databind.ObjectMapper
import com.fasterxml.jackson.databind.node.ObjectNode
import com.goatpacker.web.config.AppProperties
import com.goatpacker.web.config.PlatformProperties
import com.goatpacker.web.dto.CreatedJobDto
import com.goatpacker.web.dto.JobRequest
import com.goatpacker.web.dto.ProfileDto
import com.goatpacker.web.entity.ArtifactEntity
import com.goatpacker.web.entity.BlobEntity
import com.goatpacker.web.entity.PrincipalEntity
import com.goatpacker.web.entity.PrincipalKind
import com.goatpacker.web.entity.QueueJobEntity
import com.goatpacker.web.entity.QueueStatus
import com.goatpacker.web.entity.UploadEntity
import com.goatpacker.web.entity.UploadState
import com.goatpacker.web.exception.ApiException
import com.goatpacker.web.exception.ErrorCode
import com.goatpacker.web.exception.ErrorMessages
import com.goatpacker.web.exception.badRequest
import com.goatpacker.web.exception.expired
import com.goatpacker.web.exception.notFound
import com.goatpacker.web.exception.underMaintenance
import com.goatpacker.web.model.JobKind
import com.goatpacker.web.model.ProjectBed
import com.goatpacker.web.model.ScheduleDefaults
import com.goatpacker.web.repository.ArtifactRepository
import com.goatpacker.web.repository.BlobRepository
import com.goatpacker.web.repository.OperationalStateRepository
import com.goatpacker.web.repository.PlatformRepository
import com.goatpacker.web.repository.PrincipalRepository
import com.goatpacker.web.repository.QueueJobRepository
import com.goatpacker.web.repository.UploadRepository
import com.goatpacker.web.security.Secrets
import com.goatpacker.web.service.CustomProfile
import com.goatpacker.web.service.ProfileService
import com.goatpacker.web.service.SlicerChoice
import com.goatpacker.web.service.StlInputs
import com.goatpacker.web.service.runner.JobArtifacts
import com.goatpacker.web.util.SECONDS_PER_DAY
import org.springframework.context.annotation.Profile
import org.springframework.http.HttpStatus
import org.springframework.stereotype.Service
import org.springframework.transaction.annotation.Transactional
import java.nio.file.Files
import java.time.Instant

/** Постановка задач в очередь: упаковка, расписание и калибровка из загруженного файла, а также проверка нарезкой. */
@Service
@Profile("server")
class JobCreationService(
    private val jobs: QueueJobRepository,
    private val owners: PrincipalRepository,
    private val uploads: UploadRepository,
    private val blobs: BlobRepository,
    private val artifacts: ArtifactRepository,
    private val platform: PlatformRepository,
    private val operationalState: OperationalStateRepository,
    private val profiles: ProfileService,
    private val admission: JobAdmissionService,
    private val jobAccess: JobAccessService,
    private val limiter: RateLimiter,
    private val mapper: ObjectMapper,
    private val props: PlatformProperties,
    private val app: AppProperties,
) {
    /** Вход задачи: блоб с файлом, имя для результатов и (если файл загружен) сама загрузка. */
    private class JobInput(val blobId: String, val originalName: String, val upload: UploadEntity?, val isStl: Boolean)

    /** Снимок профиля задачи и действующий слайсер STL (null — вход 3MF). */
    private data class ProfileSnapshot(val dto: ProfileDto, val json: String, val slicer: String?)

    /** Ставит задачу в очередь. Повтор с тем же ключом идемпотентности и теми же параметрами возвращает ту же задачу. */
    @Transactional
    fun create(ownerId: String, key: String, request: JobRequest): CreatedJobDto {
        requireIdempotencyKey(key)
        val kind = JobRequestValidator.validate(request)
        // Старые клиенты: PACK по packJobId = проверка нарезкой внутри исходной задачи, ответ — id родителя.
        if (kind == JobKind.PACK && request.packJobId != null) return createCheck(ownerId, key, request.packJobId)

        val owner = lockOwner(ownerId)
        val hash = Secrets.hash(mapper.writeValueAsString(request))
        findIdempotent(ownerId, key, hash)?.let { return CreatedJobDto(it.id) }

        val input = resolveInput(request, kind, ownerId)
        limitJobsPerDay(owner)
        val budgetDay = admission.admit(owner)
        val (profile, profileJson, slicer) = snapshotProfile(request, input)
        requireOptionsSupported(request, kind, profile, owner)

        val token = Secrets.token()
        val job = jobs.save(
            QueueJobEntity(
                ownerId = ownerId,
                kind = kind,
                inputBlobId = input.blobId,
                originalName = input.originalName,
                profileId = profile.id,
                profileJson = profileJson,
                // Слайсер STL — действующий (выбранный или умолчание профиля): воркер передаёт его пакеру.
                paramsJson = mapper.writeValueAsString(request.copy(slicer = slicer)),
                requestKey = key,
                requestHash = hash,
                readTokenHash = Secrets.hash(token),
                reservedBytes = props.outputReservationBytes,
                computeReservedSeconds = props.jobTimeoutSeconds,
                budgetDay = budgetDay,
            ),
        )
        return CreatedJobDto(job.id, token)
    }

    /**
     * «Проверить нарезкой»: полная нарезка стола, собранного без неё, внутри той же задачи. Это внутренняя
     * дочерняя строка очереди (воркер и пакер те же: PACK с входом, профилем и параметрами родителя,
     * finalSlice=true и состоянием родителя), привязанная к родителю; наружу её id не отдаётся.
     * Не считается новой задачей: в суточный лимит не идёт (вычислительный бюджет резервируется как обычно).
     */
    @Transactional
    fun createCheck(ownerId: String, key: String, parentId: String): CreatedJobDto {
        requireIdempotencyKey(key)
        val owner = lockOwner(ownerId)
        val hash = Secrets.hash("check:$parentId")
        jobs.findByOwnerIdAndRequestKey(ownerId, key)?.let {
            if (it.requestHash != hash || it.parentJobId != parentId) idempotencyConflict()
            return CreatedJobDto(parentId)
        }
        val parent = jobAccess.findOwned(parentId, ownerId)
        if (parent.parentJobId != null || parent.status != QueueStatus.SUCCEEDED || parent.kind != JobKind.PACK) {
            badRequest("Нарезать можно только успешную упаковку")
        }
        parent.checkJobId?.let { jobs.findById(it).orElse(null) }?.let { previous ->
            if (previous.status in QueueStatus.ACTIVE) badRequest("Проверка нарезкой уже идёт")
            if (previous.status == QueueStatus.SUCCEEDED) badRequest("Стол уже проверен нарезкой")
        }
        val state = artifacts.findAllByJobId(parent.id).find { it.name == JobArtifacts.PACK_STATE }
            ?: badRequest("Эта упаковка уже нарезана целиком")
        val stateBlobId = promoteToBlob(state)
        val budgetDay = admission.admit(owner)

        val params = mapper.readValue(parent.paramsJson, JobRequest::class.java).copy(finalSlice = true, finishStateBlobId = stateBlobId)
        val child = jobs.save(
            QueueJobEntity(
                ownerId = ownerId,
                kind = JobKind.PACK,
                inputBlobId = parent.inputBlobId,
                originalName = parent.originalName,
                profileId = parent.profileId,
                profileJson = parent.profileJson,
                paramsJson = mapper.writeValueAsString(params),
                requestKey = key,
                requestHash = hash,
                readTokenHash = null,
                reservedBytes = props.outputReservationBytes,
                computeReservedSeconds = props.jobTimeoutSeconds,
                budgetDay = budgetDay,
                parentJobId = parent.id,
            ),
        )
        parent.checkJobId = child.id
        return CreatedJobDto(parent.id)
    }

    private fun requireIdempotencyKey(key: String) {
        if (!IDEMPOTENCY_KEY.matches(key)) badRequest("Нужен Idempotency-Key длиной 8–100")
    }

    private fun lockOwner(ownerId: String): PrincipalEntity {
        platform.lockForUpdate()
        val owner = owners.findByIdForUpdate(ownerId) ?: notFound()
        if (operationalState.isMaintenance()) underMaintenance("Сервис на обслуживании")
        return owner
    }

    /** Ранее поставленная задача с этим ключом: тот же запрос — вернуть её, другие параметры — конфликт. */
    private fun findIdempotent(ownerId: String, key: String, hash: String): QueueJobEntity? {
        val existing = jobs.findByOwnerIdAndRequestKey(ownerId, key) ?: return null
        if (existing.requestHash != hash) idempotencyConflict()
        return existing
    }

    private fun idempotencyConflict(): Nothing =
        throw ApiException(HttpStatus.CONFLICT, ErrorCode.CONFLICT, ErrorMessages.IDEMPOTENCY_KEY_CONFLICT)

    /** Гостю и пользователю разное число новых задач в сутки; проверка нарезкой сюда не попадает. */
    private fun limitJobsPerDay(owner: PrincipalEntity) {
        val perDay = props.jobsPerDay(owner.kind)
        val message = if (owner.kind == PrincipalKind.GUEST) {
            "Гостю доступно $perDay задачи в сутки — войдите или зарегистрируйтесь, чтобы запускать больше"
        } else {
            "Достигнут суточный лимит: $perDay задач — повторите завтра"
        }
        limiter.check("jobs:${owner.id}", RateRule(perDay, SECONDS_PER_DAY), message)
    }

    private fun resolveInput(request: JobRequest, kind: JobKind, ownerId: String): JobInput =
        if (request.fileId != null) uploadedInput(request, kind, ownerId) else packedInput(request.packJobId!!, request.profileId, ownerId)

    private fun uploadedInput(request: JobRequest, kind: JobKind, ownerId: String): JobInput {
        val file = uploads.findById(request.fileId!!).orElse(null)?.takeIf { it.ownerId == ownerId && it.state == UploadState.READY }
            ?: notFound()
        if (file.expiresAt?.isBefore(Instant.now()) == true) expired("Файл истёк")
        // STL — только вход упаковки из загруженного файла. Расписание по упаковке STL получает её
        // packed.3mf: имя источника становится .3mf, иначе воркер положил бы проект как input.stl.
        val isStl = StlInputs.isStl(file.originalName)
        if (isStl && kind != JobKind.PACK) badRequest("STL можно только упаковать: для расписания и калибровки нужен проект .3mf")
        // Каталожный принтер для 3MF — только распознанный по файлу: пакер сверяет printer_model проекта с
        // printer_name профиля. Нераспознанный 3MF упаковывается лишь со своим столом (CustomProfile),
        // распознанный — только своим профилем (разовый стол спорил бы с каталогом).
        // STL — любой принтер каталога: source_project запишет в проект его printer_model.
        if (!isStl && request.customBed != null && file.profileId != null) badRequest(CustomProfile.RECOGNISED_PRINTER)
        if (!isStl && request.customBed == null && file.profileId != request.profileId) {
            badRequest("Принтер файла не соответствует выбранному профилю")
        }
        return JobInput(file.blobId!!, file.originalName, file, isStl)
    }

    /** Расписание по готовой упаковке: её packed.3mf становится входом (STL-упаковка отдаёт проект, имя меняется на .3mf). */
    private fun packedInput(packJobId: String, profileId: String, ownerId: String): JobInput {
        val previous = jobAccess.findOwned(packJobId, ownerId)
        if (previous.status != QueueStatus.SUCCEEDED || previous.kind != JobKind.PACK || previous.profileId != profileId) {
            badRequest("Нужна успешная упаковка того же принтера")
        }
        val packed = artifacts.findAllByJobId(previous.id).find { it.name == JobArtifacts.PACKED_3MF } ?: notFound()
        return JobInput(promoteToBlob(packed), StlInputs.projectName(previous.originalName), null, false)
    }

    /** Артефакт задачи как неизменяемый блоб-вход новой задачи: не зависит от судьбы родителя (GC не тронет). */
    private fun promoteToBlob(artifact: ArtifactEntity): String =
        blobs.findByObjectKey(artifact.objectKey)?.id
            ?: blobs.save(BlobEntity(sha256 = "$ARTIFACT_BLOB_PREFIX${artifact.id}", sizeBytes = artifact.sizeBytes, objectKey = artifact.objectKey)).id

    /**
     * Профиль собирается при постановке и хранится снимком в задаче (воркер лишь пишет его в profile.json):
     * каталожный — как есть, свой стол — разовый профиль из размеров пользователя и стола проекта; для STL —
     * под выбранный слайсер ([SlicerChoice]).
     */
    private fun snapshotProfile(request: JobRequest, input: JobInput): ProfileSnapshot {
        val profile = request.customBed?.let { bed ->
            val projectBed = input.upload?.projectBedJson?.let { mapper.readValue(it, ProjectBed::class.java) }
            val printerModel = if (input.isStl) CustomProfile.NAME else input.upload?.printerModel
            profiles.custom(bed, projectBed, printerModel).let { it.dto to it.json }
        } ?: profiles.require(request.profileId).let { it.dto to Files.readString(it.file) }
        // Слайсер STL-задачи — в снимок: по нему воркер берёт образ, source_project штампует проект.
        val slicer = SlicerChoice.resolve(profile.first, request.slicer, input.isStl)
        val node = mapper.readTree(profile.second) as ObjectNode
        val snapshot = if (slicer != null && SlicerChoice.apply(node, slicer)) mapper.writeValueAsString(node) else profile.second
        // Принтер без пресета машины (orca_machine) из STL не собрать — отказ сразу, а не в воркере.
        if (input.isStl) {
            StlInputs.printerArgs(node)
            if (props.localRunner) StlInputs.localTools(app, node)
        }
        return ProfileSnapshot(profile.first, snapshot, slicer)
    }

    private fun requireOptionsSupported(request: JobRequest, kind: JobKind, profile: ProfileDto, owner: PrincipalEntity) {
        if (request.nozzleClean.isNotBlank() && !profile.supportsNozzleClean) badRequest("Принтер не поддерживает чистку сопла")
        if (request.amsCorrection && profile.filamentChangeSeconds == null) badRequest("Для этого принтера коррекция смены прутка не нужна")
        if (kind == JobKind.CALIBRATION && profile.filamentChangeSeconds == null) badRequest(ProfileService.CALIBRATION_NOT_NEEDED)
        if (owner.kind == PrincipalKind.GUEST && request.benchMode == ScheduleDefaults.BENCH_MODE_FULL) {
            badRequest("Полный перебор партий доступен только аккаунту")
        }
    }

    private companion object {
        val IDEMPOTENCY_KEY = Regex("[a-zA-Z0-9_-]{8,100}")

        /** Блоб-вход, созданный из артефакта, а не из загрузки: настоящей контрольной суммы у него нет. */
        const val ARTIFACT_BLOB_PREFIX = "artifact:"
    }
}
