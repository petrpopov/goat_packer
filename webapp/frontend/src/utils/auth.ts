import type { LoginStep } from './routes';

/** Бэкенд требует пароль 12–72 символа (те же лимиты, что были в AccountPage). */
export const PASSWORD_MIN = 12;
export const PASSWORD_MAX = 72;

export interface AuthFields {
  login?: string;
  password?: string;
  token?: string;
  /** Поле-заготовка регистрации: бэкенд email пока не принимает, не отправляется. */
  email?: string;
}

/**
 * Запрос к /api/v2/auth/* для шага формы. null — заготовка без сервера: письма для сброса
 * пароля пока не отправляются, экран «Проверьте почту» показывается без запроса.
 */
export function authRequest(
  step: LoginStep,
  fields: AuthFields
): { path: string; body: Record<string, string> } | null {
  const password = fields.password ?? '';
  switch (step) {
    case 'signin':
      return { path: '/auth/login', body: { login: (fields.login ?? '').trim(), password } };
    case 'register':
      return { path: '/auth/register', body: { login: (fields.login ?? '').trim(), password } };
    case 'code':
      return { path: '/auth/reset', body: { token: (fields.token ?? '').trim(), password } };
    default:
      return null;
  }
}
