/** Контракт API бэкенда Goat Packer. */

export type SlicerKind = string;

export interface PrinterProfile {
  id: string;
  name: string;
  /** Производитель для группировки в выборе принтера; бэкенд отдаёт всегда, опционально ради старых ответов (запасной путь — utils/printers vendorOf). */
  vendor?: string;
  slicerKind: SlicerKind;
  bedWidthMm: number;
  bedDepthMm: number;
  edgeMarginMm: number;
  supportsNozzleClean: boolean;
  /** Реальная длительность смены прутка AMS, с; нет/null — принтеру коррекция не нужна (тулчейнджер). */
  filamentChangeSeconds?: number | null;
  /** [x0, y0, x1, y1], мм. */
  printableArea: number[];
  exclusionRectangles: number[][];
  /** Упрощённый контур пластины; null — прямоугольник области. */
  plateOutline: number[][] | null;
}

/** Ответ /api/profiles/detect. printerModel — принтер, записанный в файле (для понятных ошибок). */
export interface DetectProfileResponse {
  profileId: string | null;
  printerModel: string | null;
  /** 'stl' — сетка без настроек слайсера: принтер выбирает пользователь. Нет поля — старый бэкенд, 3MF. */
  kind?: UploadKind;
  /** Стол из настроек 3MF, когда принтер не распознан (profileId = null); иначе null/нет. */
  projectBed?: ProjectBed | null;
}

/** Стол, прочитанный из project_settings.config 3MF неизвестного принтера (мм, координаты стола). */
export interface ProjectBed {
  /** [x0, y0, x1, y1]. */
  printableArea: number[];
  /** Прямоугольники [x0, y0, x1, y1]; многоугольник из проекта — его габарит. */
  exclusionRectangles: number[][];
  /** null — высоты в проекте нет. */
  printableHeight: number | null;
}

export type UploadKind = '3mf' | 'stl';

export type JobKind = 'pack' | 'schedule' | 'calibration';

export type JobStatus =
  'QUEUED' | 'RUNNING' | 'FINALIZING' | 'LOST' | 'RETRY_WAIT' | 'SUCCEEDED' | 'FAILED' | 'CANCELLED' | 'EXPIRED';

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
  | 'sample_slice'
  | 'assembling'
  | 'final_slice'
  | 'gcode_check'
  | 'repacking'
  | 'window_slice'
  | 'writing';

/** Пройденный (или идущий: endedAtMs null) заход в этап; время — часы исполнителя. */
export interface StageSpan {
  id: PackStageId | string;
  attempt: number;
  startedAtMs: number;
  endedAtMs: number | null;
}

/** Этап упаковки: текущий, пока задача идёт, и последний — после её завершения. */
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
  /** Все заходы в этапы по порядку (у старых задач поля нет). */
  history?: StageSpan[];
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
  /** false — стол упакован по пробной нарезке, полная нарезка ещё не запускалась; нет поля — нарезан. */
  finalSliced?: boolean;
  /** Башня очистки, которую поставил пакер; null/нет поля — башни нет (или старый бэкенд). */
  tower?: PackTowerInfo | null;
}

/** Угол башни очистки (wipe_tower_x/y) и её размер, мм, координаты стола. */
export interface PackTowerInfo {
  anchorXMm: number;
  anchorYMm: number;
  widthMm: number | null;
  depthMm: number | null;
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

/** layout-событие пакера: контур (мм, вокруг центра габарита модели) и позиции копий. */
export interface PackLayout {
  source: 'pack' | 'repack' | 'window';
  objects: number;
  /** usable bed [x0, y0, x1, y1], мм. */
  bed: number[];
  keepouts: number[][];
  tower: number[] | null;
  /** Замкнутые кольца even-odd, крупнейшее первым. */
  footprint: [number, number][][];
  /** [x_mm, y_mm, angle_deg] — центр габарита модели и поворот против часовой. */
  placements: [number, number, number][];
  /** Допуск упрощения контура (RDP), мм. */
  epsilon_mm?: number;
}

/** Параметры упаковки задачи, как их отправили (бэкенд: PackParamsDto); у доводки — родителя. */
export interface PackParamsView {
  clearance: number;
  resolution: number;
  layout: 'search' | 'grid';
  /** Спецификация чистки сопла пакера; '' — выключена. */
  nozzleClean: string;
  edgeMargin: number | null;
  finalSlice: boolean;
  timeWindowHours: number | null;
  timeMarginPct: number | null;
  amsCorrection: boolean;
  changeSeconds: number | null;
  supportOverlapMm: number | null;
  customBed: { width: number; depth: number; height: number; margin: number } | null;
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
  /** Принтер pack-задачи (как в /profiles; свой стол — id custom); null — не упаковка или профиль исчез. */
  profile?: PrinterProfile | null;
  params?: PackParamsView | null;
  schedule: ScheduleInfo | null;
  calibration?: CalibrationInfo | null;
  stage?: JobStage | null;
  /** Последняя раскладка пакера (после проверки нарезкой — её); остаётся и после успеха. */
  layout?: PackLayout | null;
  /** Проверка нарезкой этой упаковки; null — не запускалась. */
  check?: PackCheck | null;
  /** Когда клиент получил этот ответ (Date.now()); проставляет useJob для локального отсчёта времени. */
  receivedAtMs?: number;
}

/** Проверка нарезкой внутри задачи упаковки (полная нарезка стола, упакованного без неё). */
export interface PackCheck {
  status: JobStatus;
  queuePosition?: number | null;
  progressPercent: number;
  phaseMessage: string | null;
  error: string | null;
  startedAt?: string | null;
  finishedAt: string | null;
  stage?: JobStage | null;
  /** Итог проверки (стол мог поменяться: сдвиг или перепаковка); null — проверка не завершилась. */
  pack: PackJobInfo | null;
  /** Файлы проверки; ссылки — через id задачи. */
  artifacts: JobArtifact[];
}

export interface StartJobResponse {
  jobId: string;
}

// ───── Серверный режим: сессия, «Мои задачи», админка (/api/v2) ─────

export interface Session {
  id: string;
  kind: 'GUEST' | 'USER';
  login: string | null;
  role: 'USER' | 'ADMIN';
  storageQuotaBytes: number;
}

export interface PageOf<T> {
  items: T[];
  total: number;
}

/** Исходник, загруженный в профиль (RECEIVING — ещё принимается). */
export interface UploadedFile {
  fileId: string;
  originalName: string;
  sizeBytes: number;
  state: string;
}

export interface AdminUser extends Session {
  blocked: boolean;
  blockReason: string | null;
  createdAt: string;
  lastSeenAt: string;
  storageUsedBytes: number;
  resetRequired: boolean;
}

export interface WorkerState {
  id: string;
  cpuLimit: number;
  memoryLimitBytes: number;
  capacity: number;
  activeCount: number;
  lastSeenAt: string;
  mode: string;
}

export interface SystemState {
  configuredParallelism: number;
  effectiveParallelism: number;
  paused: boolean;
  activeJobs: number;
  queuedJobs: number;
  workers: WorkerState[];
  storageUsedBytes: number;
  storageLimitBytes: number;
  apiAvailableProcessors: number;
  apiHeapMaxBytes: number;
}

export interface AuditEntry {
  id: string;
  action: string;
  targetId: string | null;
  details: string;
  createdAt: string;
}

/** Фильтр списка пользователей в админке: '' — все. */
export type UserKindFilter = '' | 'USER' | 'GUEST';
