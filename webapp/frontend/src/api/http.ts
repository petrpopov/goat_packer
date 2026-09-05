const API_PREFIX = '/api';

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
  const headers = isFormData ? options.headers : { 'Content-Type': 'application/json', ...(options.headers ?? {}) };

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
  get: <T>(path: string) => request<T>(path),
  postForm: <T>(path: string, formData: FormData) => request<T>(path, { method: 'POST', body: formData }),
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
