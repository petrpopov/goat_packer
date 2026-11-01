/** Статусы задач и разбор их ошибок — общие для упаковки, расписания, калибровки и истории. */

import { ApiError } from '../api/http';
import type { JobStatus } from '../api/types';

/** Задача ещё не завершилась: опрашиваем, форма заблокирована, удалить нельзя. */
export const ACTIVE_STATUSES: ReadonlySet<JobStatus> = new Set<JobStatus>([
  'QUEUED',
  'RUNNING',
  'FINALIZING',
  'LOST',
  'RETRY_WAIT',
]);

export const isActiveStatus = (status: JobStatus | null | undefined): boolean =>
  status != null && ACTIVE_STATUSES.has(status);

/** Исполнитель работает над задачей (не очередь и не повтор). */
export const isRunningStatus = (status: JobStatus | null | undefined): boolean =>
  status === 'RUNNING' || status === 'FINALIZING';

/** Задачи больше нет: удалена, сервер перезапущен (локальный режим) или истёк срок хранения. */
export const isJobGone = (error: unknown): boolean => error instanceof ApiError && [404, 410].includes(error.status);

/** Разделитель «заголовок / сырые детали» ошибки (CliErrorTranslator бэкенда). */
export const ERROR_DETAILS_SEPARATOR = '\n— детали —\n';

/** Ошибка задачи → человекопонятный заголовок и сырой хвост журнала (null — хвоста нет). */
export function splitJobError(error: string): { headline: string; details: string | null } {
  const at = error.indexOf(ERROR_DETAILS_SEPARATOR);
  return at < 0
    ? { headline: error, details: null }
    : { headline: error.slice(0, at), details: error.slice(at + ERROR_DETAILS_SEPARATOR.length) };
}

/** Сообщение сервера (на русском) или запасной текст: сетевые ошибки браузера пользователю не показываем. */
export function apiErrorMessage(error: unknown, fallback: string): string {
  return error instanceof ApiError ? error.message : fallback;
}

/** Сообщение любой ошибки (свои Error тоже на русском) или запасной текст. */
export function errorMessage(error: unknown, fallback: string): string {
  return error instanceof Error && error.message ? error.message : fallback;
}
