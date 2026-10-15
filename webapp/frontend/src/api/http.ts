const API_PREFIX = '/api';
export const serverMode = import.meta.env?.VITE_SERVER_MODE === 'true';

export class ApiError extends Error {
  readonly code: string | null;
  readonly status: number;

  constructor(message: string, code: string | null, status: number) {
    super(message);
    this.name = 'ApiError';
    this.code = code;
    this.status = status;
  }
}

/** Тело ошибки бэкенда: {"code","message","error"} — message на русском. */
interface ErrorBody {
  code?: string;
  message?: string;
  error?: string;
}

async function request<T>(path: string, options: RequestInit = {}): Promise<T> {
  const isFormData = options.body instanceof FormData;
  const headers = new Headers(
    isFormData ? options.headers : { 'Content-Type': 'application/json', ...(options.headers ?? {}) }
  );
  if (typeof document !== 'undefined' && !['GET', 'HEAD'].includes(options.method ?? 'GET')) {
    const token = document.cookie
      .split('; ')
      .find((c) => c.startsWith('XSRF-TOKEN='))
      ?.slice('XSRF-TOKEN='.length);
    if (token) headers.set('X-XSRF-TOKEN', decodeURIComponent(token));
  }

  const response = await fetch(`${API_PREFIX}${path}`, { ...options, headers });

  if (!response.ok) {
    const body = (await response.json().catch(() => null)) as ErrorBody | null;
    const message = body?.message || body?.error || `HTTP ${response.status}`;
    throw new ApiError(message, body?.code ?? null, response.status);
  }

  if (response.status === 204) {
    return null as T;
  }

  // Эндпоинты без тела (200 с пустым ответом) — не парсим как JSON.
  const text = await response.text();
  if (!text) {
    return null as T;
  }

  return JSON.parse(text) as T;
}

export const api = {
  get: <T>(path: string, signal?: AbortSignal) => request<T>(path, { signal }),
  postForm: <T>(path: string, formData: FormData, signal?: AbortSignal) =>
    request<T>(path, { method: 'POST', body: formData, signal }),
  delete: (path: string) => request<void>(path, { method: 'DELETE' }),
  /** Триггерит сохранение файла через скрытый <a download>. */
  download: (url: string, filename?: string) => {
    const link = document.createElement('a');
    link.href = url;
    link.download = filename ?? url.split('/').pop() ?? 'file';
    document.body.appendChild(link);
    link.click();
    link.remove();
  },
};

export const serverApi = {
  get: <T>(path: string, signal?: AbortSignal) => request<T>(`/v2${path}`, { signal }),
  post: <T>(path: string, data: unknown = {}, signal?: AbortSignal) =>
    request<T>(`/v2${path}`, { method: 'POST', body: JSON.stringify(data), signal }),
  putFile: <T>(path: string, file: File, signal?: AbortSignal) =>
    request<T>(`/v2${path}`, {
      method: 'PUT',
      body: file,
      headers: { 'Content-Type': 'application/octet-stream' },
      signal,
    }),
  createJob: <T>(data: unknown, key: string) =>
    request<T>('/v2/jobs', { method: 'POST', body: JSON.stringify(data), headers: { 'Idempotency-Key': key } }),
  delete: (path: string) => request<void>(`/v2${path}`, { method: 'DELETE' }),
};
