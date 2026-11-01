/** API серверного режима (/api/v2): сессия, «Мои задачи», загрузки и админка. */

import { ApiError, serverApi } from './http';
import type {
  AdminUser,
  AuditEntry,
  JobStatusResponse,
  PageOf,
  Session,
  SystemState,
  UploadedFile,
  UserKindFilter,
} from './types';

/** Адрес задачи с ключом просмотра: /pack/<id> или старый /jobs/<id>. */
const JOB_PATH = /^\/(?:pack|jobs)\/([a-z0-9-]+)$/;

/** Сессия: CSRF-cookie, текущий пользователь или новая гостевая сессия. */
export async function loadSession(): Promise<Session> {
  await serverApi.get('/auth/csrf');
  let user: Session;
  try {
    user = await serverApi.get<Session>('/auth/me');
  } catch (error) {
    if (!(error instanceof ApiError) || error.status !== 401) throw error;
    user = await serverApi.post<Session>('/auth/guest');
  }
  // Ссылка на задачу с ключом просмотра (#access=…): ключ меняется на доступ этой сессии и убирается из адреса.
  const jobId = location.pathname.match(JOB_PATH)?.[1];
  const access = new URLSearchParams(location.hash.slice(1)).get('access');
  if (jobId && access) {
    await serverApi.post('/access', { jobId, token: access });
    history.replaceState(null, '', location.pathname + location.search);
  }
  return user;
}

export const logout = () => serverApi.post('/auth/logout');

export const listJobs = (page: number) => serverApi.get<PageOf<JobStatusResponse>>(`/jobs?page=${page}`);
export const removeJob = (id: string) => serverApi.delete(`/jobs/${id}`);

export const listUploads = () => serverApi.get<UploadedFile[]>('/uploads');
export const removeUpload = (id: string) => serverApi.delete(`/uploads/${id}`);

/** Ключи кэша админки: действия сбрасывают свои списки и журнал. */
export const ADMIN_QUERY_KEYS = {
  system: ['admin-system'],
  users: ['admin-users'],
  audit: ['admin-audit'],
} as const;

/** Длина причины административного действия (бэкенд пишет её в журнал). */
export const ADMIN_REASON_LENGTH = { min: 3, max: 500 } as const;

export const fetchSystemState = () => serverApi.get<SystemState>('/admin/system');
export const updateSystemSettings = (settings: { maxParallel: number; paused: boolean; reason: string }) =>
  serverApi.post('/admin/system', settings);

export const listUsers = (kind: UserKindFilter, query: string, page: number) =>
  serverApi.get<PageOf<AdminUser>>(`/admin/users?kind=${kind}&query=${encodeURIComponent(query)}&page=${page}`);
export const setUserBlocked = (id: string, blocked: boolean, reason: string) =>
  serverApi.post(`/admin/users/${id}/block`, { blocked, reason });
/** Одноразовый токен сброса пароля (сессии пользователя отзываются). */
export const resetUserPassword = (id: string, reason: string) =>
  serverApi.post<{ token: string }>(`/admin/users/${id}/password-reset`, { reason });

export const fetchAudit = () => serverApi.get<AuditEntry[]>('/admin/audit');
