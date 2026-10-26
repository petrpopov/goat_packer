import { uploadProof } from '../utils/challenge';
import { randomId } from '../utils/randomId';
import { api, serverApi, serverMode } from './http';
import type { DetectProfileResponse, JobStatusResponse, PrinterProfile, StartJobResponse } from './types';

export async function fetchProfiles(): Promise<PrinterProfile[]> {
  return serverMode ? serverApi.get<PrinterProfile[]>('/profiles') : api.get<PrinterProfile[]>('/profiles');
}

/** Автоопределение профиля принтера по printer_model внутри 3MF. */
export async function detectProfile(file: File, signal?: AbortSignal): Promise<DetectProfileResponse> {
  if (serverMode) {
    const intent = await serverApi.post<{ fileId: string }>(
      '/uploads',
      { name: file.name, sizeBytes: file.size, ...(await uploadProof(signal)) },
      signal
    );
    try {
      const result = await serverApi.putFile<DetectProfileResponse>(`/uploads/${intent.fileId}/content`, file, signal);
      uploadedFiles.set(file, intent.fileId);
      return result;
    } catch (error) {
      await serverApi.delete(`/uploads/${intent.fileId}`).catch(() => undefined);
      throw error;
    }
  }
  const form = new FormData();
  form.append('file', file);
  const result = await api.postForm<Partial<DetectProfileResponse>>('/profiles/detect', form, signal);
  return { profileId: result?.profileId ?? null, printerModel: result?.printerModel ?? null };
}

export async function startPackJob(form: FormData): Promise<StartJobResponse> {
  return serverMode ? submitServer('PACK', form) : api.postForm<StartJobResponse>('/pack/jobs', form);
}

export async function startScheduleJob(form: FormData): Promise<StartJobResponse> {
  return serverMode ? submitServer('SCHEDULE', form) : api.postForm<StartJobResponse>('/schedule/jobs', form);
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
async function submitServer(kind: 'PACK' | 'SCHEDULE', form: FormData): Promise<StartJobResponse> {
  const file = form.get('file');
  const fileId = file instanceof File ? uploadedFiles.get(file) : undefined;
  if (file instanceof File && !fileId) throw new Error('Загрузите файл и дождитесь определения принтера');
  const body: Record<string, unknown> = { kind };
  for (const [name, value] of form.entries()) {
    if (name === 'file' || name === 'seed' || name === 'candidates') continue;
    body[name] = [
      'clearance',
      'resolution',
      'attempts',
      'edgeMargin',
      'turnaroundMin',
      'prepMin',
      'timeWindowHours',
      'changeSeconds',
      'timeMarginPct',
    ].includes(name)
      ? Number(value)
      : name === 'amsCorrection'
        ? value === 'true'
        : value;
  }
  if (fileId) body.fileId = fileId;
  let key = pendingRequests.get(form);
  if (!key) {
    key = randomId();
    pendingRequests.set(form, key);
  }
  const created = await serverApi.createJob<StartJobResponse & { readToken?: string }>(body, key);
  if (created.readToken)
    sessionStorage.setItem(
      `return:${created.jobId}`,
      `${location.origin}/jobs/${created.jobId}#access=${created.readToken}`
    );
  return created;
}
