import { uploadProof } from '../utils/challenge';
import { isStlName } from '../utils/printers';
import { randomId } from '../utils/randomId';
import { api, serverApi, serverMode, type UploadProgress } from './http';
import type { DetectProfileResponse, JobArtifact, JobStatusResponse, PrinterProfile, StartJobResponse } from './types';

export async function fetchProfiles(): Promise<PrinterProfile[]> {
  return serverMode ? serverApi.get<PrinterProfile[]>('/profiles') : api.get<PrinterProfile[]>('/profiles');
}

/** Автоопределение профиля принтера по printer_model внутри 3MF. */
export async function detectProfile(
  file: File,
  signal?: AbortSignal,
  onProgress?: UploadProgress
): Promise<DetectProfileResponse> {
  if (serverMode) {
    const intent = await serverApi.post<{ fileId: string }>(
      '/uploads',
      { name: file.name, sizeBytes: file.size, ...(await uploadProof(signal)) },
      signal
    );
    try {
      const result = await serverApi.putFile<DetectProfileResponse>(
        `/uploads/${intent.fileId}/content`,
        file,
        signal,
        onProgress
      );
      uploadedFiles.set(file, intent.fileId);
      return result;
    } catch (error) {
      await serverApi.delete(`/uploads/${intent.fileId}`).catch(() => undefined);
      throw error;
    }
  }
  // Локально STL определять нечего (принтер выбирает пользователь): без лишней отправки файла.
  // На сервере загрузка нужна — задача ссылается на fileId.
  if (isStlName(file.name)) return { profileId: null, printerModel: null, kind: 'stl', projectBed: null };
  const form = new FormData();
  form.append('file', file);
  const result = await api.postForm<Partial<DetectProfileResponse>>('/profiles/detect', form, signal, onProgress);
  return {
    profileId: result?.profileId ?? null,
    printerModel: result?.printerModel ?? null,
    kind: result?.kind ?? '3mf',
    projectBed: result?.projectBed ?? null,
  };
}

export async function startPackJob(form: FormData): Promise<StartJobResponse> {
  return serverMode ? submitServer('PACK', form) : api.postForm<StartJobResponse>('/pack/jobs', form);
}

/**
 * «Проверить нарезкой»: полная нарезка стола, упакованного без неё, — внутри той же задачи (тот же
 * адрес и строка в «Моих задачах»; ответ — её же id). Ход и итог — в поле check статуса задачи.
 */
export async function checkPackJob(jobId: string): Promise<StartJobResponse> {
  const id = encodeURIComponent(jobId);
  return serverMode
    ? serverApi.postWithKey<StartJobResponse>(`/jobs/${id}/check`, randomId())
    : api.postForm<StartJobResponse>(`/pack/jobs/${id}/finish`, new FormData());
}

/** Остановить проверку нарезкой; упакованный стол задачи остаётся. */
export async function cancelPackCheck(jobId: string): Promise<void> {
  const id = encodeURIComponent(jobId);
  if (serverMode) await serverApi.post(`/jobs/${id}/check/cancel`);
  else await api.postForm(`/pack/jobs/${id}/check/cancel`, new FormData());
}

export async function startScheduleJob(form: FormData): Promise<StartJobResponse> {
  return serverMode ? submitServer('SCHEDULE', form) : api.postForm<StartJobResponse>('/schedule/jobs', form);
}

export async function startCalibrationJob(form: FormData): Promise<StartJobResponse> {
  return serverMode ? submitServer('CALIBRATION', form) : api.postForm<StartJobResponse>('/calibration/jobs', form);
}

export async function getJob(id: string, signal?: AbortSignal): Promise<JobStatusResponse> {
  return serverMode
    ? serverApi.get<JobStatusResponse>(
        `/jobs/${encodeURIComponent(id)}?debug=${new URLSearchParams(location.search).get('debug') === 'true'}`,
        signal
      )
    : api.get<JobStatusResponse>(`/jobs/${encodeURIComponent(id)}`, signal);
}

export async function deleteJob(id: string): Promise<void> {
  return serverMode
    ? serverApi.delete(`/jobs/${encodeURIComponent(id)}`)
    : api.delete(`/jobs/${encodeURIComponent(id)}`);
}

const uploadedFiles = new WeakMap<File, string>();
const pendingRequests = new WeakMap<FormData, string>();
async function submitServer(kind: 'PACK' | 'SCHEDULE' | 'CALIBRATION', form: FormData): Promise<StartJobResponse> {
  const file = form.get('file');
  const fileId = file instanceof File ? uploadedFiles.get(file) : undefined;
  if (file instanceof File && !fileId) throw new Error('Загрузите файл и дождитесь определения принтера');
  const body: Record<string, unknown> = { kind };
  // Свой стол: в локальном API — поля custom*, в серверном — объект customBed.
  const customBed: Record<string, number> = {};
  for (const [name, value] of form.entries()) {
    if (name === 'file' || name === 'seed' || name === 'candidates') continue;
    if (name.startsWith('custom')) {
      customBed[name.slice('custom'.length).toLowerCase()] = Number(value);
      continue;
    }
    body[name] = [
      'clearance',
      'resolution',
      'attempts',
      'edgeMargin',
      'towerWidth',
      'turnaroundMin',
      'prepMin',
      'timeWindowHours',
      'changeSeconds',
      'timeMarginPct',
      'supportOverlapMm',
    ].includes(name)
      ? Number(value)
      : name === 'amsCorrection' || name === 'finalSlice'
        ? value === 'true'
        : value;
  }
  if (Object.keys(customBed).length > 0) body.customBed = customBed;
  if (fileId) body.fileId = fileId;
  let key = pendingRequests.get(form);
  if (!key) {
    key = randomId();
    pendingRequests.set(form, key);
  }
  const created = await serverApi.createJob<StartJobResponse & { readToken?: string }>(body, key);
  rememberAccess(created);
  return created;
}

/**
 * Ключ просмотра задачи (сервер отдаёт его один раз, при создании): по ссылке с ним задачу видит
 * и тот, у кого нет сессии владельца. Живёт до закрытия вкладки.
 */
function rememberAccess(created: { jobId: string; readToken?: string }): void {
  if (created.readToken) sessionStorage.setItem(`access:${created.jobId}`, created.readToken);
}

/** Ссылка на рабочую область задачи; на сервере — с ключом просмотра, если он у этой вкладки есть. */
export function jobLink(jobId: string): string {
  const token = serverMode ? sessionStorage.getItem(`access:${jobId}`) : null;
  return `${location.origin}/pack/${jobId}${token ? `#access=${token}` : ''}`;
}

/** Исходный файл задачи (тот же 3MF/STL, что получил пакер): сцена задачи, открытой по ссылке. */
export async function downloadJobInput(jobId: string, name: string, signal?: AbortSignal): Promise<File> {
  const id = encodeURIComponent(jobId);
  const url = serverMode
    ? (await serverApi.post<{ url: string }>(`/jobs/${id}/input/download`, {}, signal)).url
    : `/api/jobs/${id}/input`;
  const response = await fetch(url, { signal });
  if (!response.ok) throw new Error('Не удалось получить файл задачи');
  return new File([await response.blob()], name);
}

/** Скачать артефакт: локально — прямая ссылка, на сервере — подписанная ссылка хранилища. */
export async function downloadArtifact(artifact: JobArtifact): Promise<void> {
  if (!serverMode) return api.download(artifact.downloadUrl, artifact.name);
  const { url } = await serverApi.post<{ url: string }>(artifact.downloadUrl.replace('/api/v2', ''));
  location.href = url;
}
