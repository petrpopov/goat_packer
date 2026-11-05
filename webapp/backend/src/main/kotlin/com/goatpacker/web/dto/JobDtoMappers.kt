package com.goatpacker.web.dto

import com.goatpacker.web.model.CalibrationJobResult
import com.goatpacker.web.model.Detection
import com.goatpacker.web.model.PackJobResult
import com.goatpacker.web.model.PackParams
import com.goatpacker.web.model.PackTimeResult
import com.goatpacker.web.model.PackTowerResult
import com.goatpacker.web.model.PlateRecord
import com.goatpacker.web.model.ScheduleJobRecord
import com.goatpacker.web.model.ScheduleJobResult
import com.goatpacker.web.util.ClockFormat

fun PackJobResult.toDto() = PackInfoDto(
    objectCount = objectCount,
    profileId = profileId,
    clearanceMm = clearanceMm,
    time = time?.toDto(),
    supportsDetected = supportsDetected,
    supportOverlapMm = supportOverlapMm,
    finalSliced = finalSliced,
    tower = tower?.toDto(),
    targetSlicer = targetSlicer,
)

fun PackTowerResult.toDto() = PackTowerDto(anchorXMm, anchorYMm, widthMm, depthMm)

fun PackTimeResult.toDto() = PackTimeDto(
    windowHours = windowHours,
    marginPct = marginPct,
    realChangeSeconds = realChangeSeconds,
    chosenCount = chosenCount,
    fullCount = fullCount,
    correctedSeconds = correctedSeconds,
)

fun CalibrationJobResult.toDto() = CalibrationInfoDto(
    profileId = profileId,
    variant = variant,
    changesExpected = changesExpected,
    changesSliced = changesSliced,
    estimateASeconds = estimateASeconds,
    estimateBSeconds = estimateBSeconds,
    slicerChangeSeconds = slicerChangeSeconds,
    instructions = instructions,
)

fun ScheduleJobResult.toDto() = ScheduleInfoDto(
    printer = printer,
    totalObjects = totalObjects,
    totalFilamentGrams = totalFilamentGrams,
    completionClock = completionClock,
    jobs = jobs.map { it.toDto() },
    plates = plates.map { it.toDto() },
)

fun ScheduleJobRecord.toDto() = ScheduleJobDto(
    sequence = sequence,
    batchSize = batchSize,
    startClock = ClockFormat.format(startSeconds),
    endClock = ClockFormat.format(endSeconds),
    startSeconds = startSeconds,
    endSeconds = endSeconds,
    slicerSeconds = slicerSeconds,
    preparationSeconds = preparationSeconds,
    filamentGrams = filamentGrams,
    cumulativeObjects = cumulativeObjects,
    overnight = overnight,
)

fun PlateRecord.toDto() = PlateDto(label, objectCount)

/** Настройки pack-джоба для режима «только чтение» (локальный режим). */
fun PackParams.toDto() = PackParamsDto(
    clearance = clearance,
    resolution = resolution,
    layout = layout,
    nozzleClean = nozzleClean,
    edgeMargin = edgeMargin,
    towerWidth = towerWidth,
    finalSlice = finalSlice,
    timeWindowHours = timeWindowHours,
    timeMarginPct = timeMarginPct,
    amsCorrection = amsCorrection,
    changeSeconds = changeSeconds,
    supportOverlapMm = supportOverlapMm,
    customBed = customBed,
    slicer = slicer,
)

/** Настройки pack-джоба для режима «только чтение» (серверный режим). */
fun JobRequest.toPackParamsDto() = PackParamsDto(
    clearance = clearance,
    resolution = resolution,
    layout = layout,
    nozzleClean = nozzleClean,
    edgeMargin = edgeMargin,
    towerWidth = towerWidth,
    finalSlice = finalSlice,
    timeWindowHours = timeWindowHours,
    timeMarginPct = timeMarginPct,
    amsCorrection = amsCorrection,
    changeSeconds = changeSeconds,
    supportOverlapMm = supportOverlapMm,
    customBed = customBed,
    slicer = slicer,
)

fun Detection.toDto() = DetectProfileDto(profileId, printerModel, kind, projectBed)
