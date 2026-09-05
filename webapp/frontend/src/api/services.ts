import { api } from './http';
import type { JobStatusResponse, PrinterProfile, StartJobResponse } from './types';

export async function fetchProfiles(): Promise<PrinterProfile[]> {
  return api.get<PrinterProfile[]>('/profiles');
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
