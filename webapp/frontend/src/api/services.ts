import { api } from './http';
import type { DetectProfileResponse, JobStatusResponse, PrinterProfile, StartJobResponse } from './types';

export async function fetchProfiles(): Promise<PrinterProfile[]> {
  return api.get<PrinterProfile[]>('/profiles');
}

/** Автоопределение профиля принтера по printer_model внутри 3MF. */
export async function detectProfile(file: File): Promise<DetectProfileResponse> {
  const form = new FormData();
  form.append('file', file);
  return api.postForm<DetectProfileResponse>('/profiles/detect', form);
}

export async function startPackJob(form: FormData): Promise<StartJobResponse> {
  return api.postForm<StartJobResponse>('/pack/jobs', form);
}

export async function startScheduleJob(form: FormData): Promise<StartJobResponse> {
  return api.postForm<StartJobResponse>('/schedule/jobs', form);
}

export async function getJob(id: string): Promise<JobStatusResponse> {
  return api.get<JobStatusResponse>(`/jobs/${encodeURIComponent(id)}`);
}
