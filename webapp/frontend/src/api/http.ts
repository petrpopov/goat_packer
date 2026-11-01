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

function xsrfToken(): string | null {
  if (typeof document === 'undefined') return null;
  const token = document.cookie
    .split('; ')
    .find((c) => c.startsWith('XSRF-TOKEN='))
    ?.slice('XSRF-TOKEN='.length);
  return token ? decodeURIComponent(token) : null;
}

function parseBody<T>(text: string): T {
  return (text ? JSON.parse(text) : null) as T;
}

/** Сколько отправлено из скольких, байт. */
export type UploadProgress = (sent: number, total: number) => void;

/**
 * Отправка файла с прогрессом: fetch не сообщает, сколько ушло, поэтому XHR — с теми же
 * заголовками (XSRF) и тем же разбором ошибок, что у request.
 */
function sendWithProgress<T>(
  method: 'PUT' | 'POST',
  path: string,
  body: Blob | FormData,
  onProgress?: UploadProgress,
  signal?: AbortSignal,
  contentType?: string
): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const xhr = new XMLHttpRequest();
    xhr.open(method, `${API_PREFIX}${path}`);
    if (contentType) xhr.setRequestHeader('Content-Type', contentType);
    const token = xsrfToken();
    if (token) xhr.setRequestHeader('X-XSRF-TOKEN', token);
    xhr.upload.onprogress = (event) => {
      if (event.lengthComputable) onProgress?.(event.loaded, event.total);
    };
    xhr.onload = () => {
      if (xhr.status >= 200 && xhr.status < 300) {
        try {
          resolve(parseBody<T>(xhr.responseText));
        } catch (error) {
          reject(error);
        }
        return;
      }
      let body: ErrorBody | null = null;
      try {
        body = JSON.parse(xhr.responseText) as ErrorBody;
      } catch {
        body = null;
      }
      reject(new ApiError(body?.message || body?.error || `HTTP ${xhr.status}`, body?.code ?? null, xhr.status));
    };
    xhr.onerror = () => reject(new TypeError('Сеть недоступна'));
    xhr.onabort = () => reject(new DOMException('Загрузка отменена', 'AbortError'));
    if (signal) {
      if (signal.aborted) return xhr.abort();
      signal.addEventListener('abort', () => xhr.abort(), { once: true });
    }
    xhr.send(body);
  });
}

async function request<T>(path: string, options: RequestInit = {}): Promise<T> {
  const isFormData = options.body instanceof FormData;
  const headers = new Headers(
    isFormData ? options.headers : { 'Content-Type': 'application/json', ...(options.headers ?? {}) }
  );
  if (!['GET', 'HEAD'].includes(options.method ?? 'GET')) {
    const token = xsrfToken();
    if (token) headers.set('X-XSRF-TOKEN', token);
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
  return parseBody<T>(await response.text());
}

export const api = {
  get: <T>(path: string, signal?: AbortSignal) => request<T>(path, { signal }),
  postForm: <T>(path: string, formData: FormData, signal?: AbortSignal, onProgress?: UploadProgress) =>
    onProgress
      ? sendWithProgress<T>('POST', path, formData, onProgress, signal)
      : request<T>(path, { method: 'POST', body: formData, signal }),
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
  putFile: <T>(path: string, file: File, signal?: AbortSignal, onProgress?: UploadProgress) =>
    sendWithProgress<T>('PUT', `/v2${path}`, file, onProgress, signal, 'application/octet-stream'),
  /** POST с ключом идемпотентности (повтор того же запроса не создаёт второй задачи). */
  postWithKey: <T>(path: string, key: string, data: unknown = {}) =>
    request<T>(`/v2${path}`, { method: 'POST', body: JSON.stringify(data), headers: { 'Idempotency-Key': key } }),
  createJob: <T>(data: unknown, key: string) =>
    request<T>('/v2/jobs', { method: 'POST', body: JSON.stringify(data), headers: { 'Idempotency-Key': key } }),
  delete: (path: string) => request<void>(`/v2${path}`, { method: 'DELETE' }),
};
