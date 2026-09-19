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
}

/** Ответ /api/profiles/detect. */
export interface DetectProfileResponse {
  profileId: string | null;
}

export type JobKind = 'pack' | 'schedule';

export type JobStatus = 'QUEUED' | 'RUNNING' | 'SUCCEEDED' | 'FAILED';

export type JobPhase =
  'probe' | 'packing' | 'assembling' | 'slicing' | 'repacking' | 'benchmark' | 'optimizing' | 'writing' | 'cleanup';

export interface JobArtifact {
  name: string;
  sizeBytes: number;
  downloadUrl: string;
}

export interface PackJobInfo {
  objectCount: number;
  profileId: string;
  clearanceMm: number;
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
}

export interface StartJobResponse {
  jobId: string;
}
