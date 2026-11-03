/** Статусы задач и разбор их ошибок — общие для упаковки, расписания, калибровки и истории. */

import { ApiError } from '../api/http';
import type { JobStatus } from '../api/types';
import { type PrinterLike, vendorOf } from './printers';

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

/** Подсказка к предварительному числу объектов: раскладка готова, но пакер ещё может её изменить. */
export const PRELIMINARY_HINT = 'Предварительно: по последней раскладке, после нарезок число может измениться';

/**
 * Bambu Studio и Snapmaker Orca при открытии проекта сами переставляют башню очистки (GUI зажимает её
 * в свой диапазон), и она может встать на объекты. Пакер оставил место под башню в выбранном углу —
 * пользователь возвращает её туда руками.
 */
export const TOWER_GUI_WARNING =
  'При открытии в Bambu Studio или Snapmaker Orca башня очистки сдвинется — перетащите её обратно в угол, где она стоит на этом столе: место под неё оставлено.';

/** Предупреждение про башню у готового стола; null — задача не завершилась или башни на столе нет. */
export function towerWarning(
  status: JobStatus | null | undefined,
  result: { tower?: unknown } | null | undefined
): string | null {
  return status === 'SUCCEEDED' && result?.tower ? TOWER_GUI_WARNING : null;
}

/**
 * Snapmaker Orca при нарезке сравнивает линии подставок с линиями юбки башни очистки на первом слое; подставки в его окне
 * чуть отличаются от тех, по которым пакер оставил зазор, и одна из фигурок у башни может дать «Serious warning:
 * Conflicts of G-code paths … (WipeTower …)». Пакер из-за этого фигурки не убирает — пользователю говорим, что делать.
 */
export const SNAPMAKER_TOWER_CONFLICT_NOTE =
  'Snapmaker Orca может предупредить о конфликте башни очистки с соседней фигуркой на первом слое. Если так — сдвиньте башню на 1–2 мм от фигурки или удалите ближайшую фигурку.';

/** Подсказка про конфликт башни в Snapmaker Orca у готового стола; null — другой производитель, задача не завершилась или башни нет. */
export function snapmakerTowerNote(
  status: JobStatus | null | undefined,
  result: { tower?: unknown } | null | undefined,
  profile: PrinterLike | null | undefined
): string | null {
  if (status !== 'SUCCEEDED' || !result?.tower || !profile) return null;
  return vendorOf(profile).toLowerCase() === 'snapmaker' ? SNAPMAKER_TOWER_CONFLICT_NOTE : null;
}
