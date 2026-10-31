/** Контракт API бэкенда Goat Packer. */

export type SlicerKind = string;

export interface PrinterProfile {
  id: string;
  name: string;
  slicerKind: SlicerKind;
  bedWidthMm: number;
  bedDepthMm: number;
  edgeMarginMm: number;
  supportsNozzleClean: boolean;
  /** Реальная длительность смены прутка AMS, с; нет/null — принтеру коррекция не нужна (тулчейнджер). */
  filamentChangeSeconds?: number | null;
}

/** Ответ /api/profiles/detect. printerModel — принтер, записанный в файле (для понятных ошибок). */
export interface DetectProfileResponse {
  profileId: string | null;
  printerModel: string | null;
}

export type JobKind = 'pack' | 'schedule' | 'calibration';

export type JobStatus = 'QUEUED' | 'RUNNING' | 'FINALIZING' | 'LOST' | 'RETRY_WAIT' | 'SUCCEEDED' | 'FAILED' | 'CANCELLED' | 'EXPIRED';

export type JobPhase =
  | 'upload'
  | 'probe'
  | 'packing'
  | 'assembling'
  | 'slicing'
  | 'repacking'
  | 'benchmark'
  | 'optimizing'
  | 'writing'
  | 'cleanup';

/** Этапы colored_3mf_packer (stage-события протокола прогресса). */
export type PackStageId =
  | 'probe_slice'
  | 'packing'
  | 'assembling'
  | 'final_slice'
  | 'gcode_check'
  | 'repacking'
  | 'window_slice'
  | 'writing';

/** Текущий этап упаковки (только pack-задачи, пока задача выполняется). */
export interface JobStage {
  id: PackStageId | string;
  startedAtMs: number;
  /** Сколько этап идёт на момент ответа сервера, с. */
  elapsedSeconds: number;
  /** Оценка длительности этапа от пакера, с; null — оценки нет. */
  expectedSeconds: number | null;
  objects: number | null;
  /** Номер захода в этап (нарезки под окно времени повторяются). */
  attempt: number;
  /** Пройденные этапы в порядке первого захода, включая текущий. */
  visited: string[];
  /** Последняя английская строка пакера — вторичная подробность. */
  detail: string | null;
}

export interface JobArtifact {
  name: string;
  sizeBytes: number;
  downloadUrl: string;
}

/** Оценка времени записанного стола (из блока time отчёта упаковщика). */
export interface PackTimeInfo {
  /** Окно печати, ч; null — окно не задавалось. */
  windowHours?: number | null;
  marginPct: number;
  /** Реальная длительность смены, с; null — коррекция смен выключена. */
  realChangeSeconds?: number | null;
  /** Объектов на записанном столе. */
  chosenCount: number;
  /** Объектов на полном (максимальном) столе. */
  fullCount: number;
  /** Скорректированное время печати записанного стола, с. */
  correctedSeconds: number;
}

export interface PackJobInfo {
  objectCount: number;
  profileId: string;
  clearanceMm: number;
  time?: PackTimeInfo | null;
  /** У модели обнаружены поддержки — поворот заблокирован; нет поля — старый пакер. */
  supportsDetected?: boolean | null;
  /** Допустимое перекрытие поддержек, мм; null — без ограничений. */
  supportOverlapMm?: number | null;
}

/** Итог задачи калибровки AMS: значения calibration.json и текст instructions.md. */
export interface CalibrationInfo {
  profileId: string;
  variant: string;
  changesExpected: number;
  changesSliced: number;
  estimateASeconds: number;
  estimateBSeconds: number;
  slicerChangeSeconds: number;
  instructions: string;
}

export interface ScheduleJobEntry {
  sequence: number;
  batchSize: number;
  startClock: string;
  endClock: string;
  startSeconds: number;
  endSeconds: number;
  slicerSeconds: number;
  preparationSeconds: number;
  filamentGrams: number;
  cumulativeObjects: number;
  overnight: boolean;
}

export interface SchedulePlate {
  label: string;
  objectCount: number;
}

export interface ScheduleInfo {
  printer: string;
  totalObjects: number;
  totalFilamentGrams: number;
  completionClock: string;
  jobs: ScheduleJobEntry[];
  plates: SchedulePlate[];
}

export interface JobStatusResponse {
  canManage?: boolean;
  originalName?: string;
  queuePosition?: number | null;
  estimatedWaitSeconds?: number | null;
  estimatedWaitFromSeconds?: number | null;
  estimatedWaitToSeconds?: number | null;
  estimateReason?: string;
  expiresAt?: string | null;
  jobId: string;
  kind: JobKind;
  status: JobStatus;
  phase: JobPhase | null;
  progressPercent: number;
  phaseMessage: string | null;
  error: string | null;
  createdAt: string;
  finishedAt: string | null;
  artifacts: JobArtifact[];
  pack: PackJobInfo | null;
  schedule: ScheduleInfo | null;
  calibration?: CalibrationInfo | null;
  stage?: JobStage | null;
  /** Когда клиент получил этот ответ (Date.now()); проставляет useJob для локального отсчёта времени. */
  receivedAtMs?: number;
}

export interface StartJobResponse {
  jobId: string;
}
