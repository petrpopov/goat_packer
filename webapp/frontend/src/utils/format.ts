/** Форматирование величин для интерфейса (числа — с точкой, единицы — на русском). */

import type { PackTimeInfo } from '../api/types';

const KIB = 1024;
const MIB = KIB * KIB;

export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return '—';
  if (bytes < KIB) return `${bytes} Б`;
  if (bytes < MIB) return `${(bytes / KIB).toFixed(1)} КБ`;
  return `${(bytes / MIB).toFixed(1)} МБ`;
}

/** Объёмы хранилища в админке и списке загрузок: «12.3 МиБ». */
export function formatMebibytes(bytes: number): string {
  return `${(bytes / MIB).toFixed(1)} МиБ`;
}

/** Целые — без дробной части, остальное — один знак: «12», «0.5». */
function formatDecimal(value: number): string {
  if (!Number.isFinite(value)) return '—';
  return Number.isInteger(value) ? String(value) : value.toFixed(1);
}

/** Секунды → минуты; целые — без дробной части. */
export function formatMinutes(seconds: number): string {
  return formatDecimal(seconds / 60);
}

/** Граммы; целые — без дробной части. */
export function formatGrams(grams: number): string {
  return formatDecimal(grams);
}

/** Секунды → «Ч ч ММ мин» (минуты округляются, всегда две цифры): оценки времени печати. */
export function formatHoursMinutes(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return '—';
  const total = Math.round(seconds / 60);
  return `${Math.floor(total / 60)} ч ${String(total % 60).padStart(2, '0')} мин`;
}

/**
 * Длительность: «45 с», «2 мин 05 с», «1 ч 03 мин». Секунды округляются вверх: этап, занявший
 * 0.3 с, — «1 с», а не «0 с» (единое правило для всех длительностей интерфейса).
 */
export function formatDuration(seconds: number): string {
  const total = Number.isFinite(seconds) ? Math.max(0, Math.ceil(seconds)) : 0;
  if (total < 60) return `${total} с`;
  const minutes = Math.floor(total / 60);
  if (minutes < 60) return `${minutes} мин ${String(total % 60).padStart(2, '0')} с`;
  return `${Math.floor(minutes / 60)} ч ${String(minutes % 60).padStart(2, '0')} мин`;
}

/** Ожидание в очереди «A–B» в минутах (нижняя граница вниз, верхняя вверх). */
export function formatWaitMinutes(fromSeconds: number, toSeconds: number): string {
  return `${Math.floor(fromSeconds / 60)}–${Math.ceil(toSeconds / 60)}`;
}

/** Строки сводки времени упакованного стола: оценка с коррекцией и, с окном, «на столе K из M». */
export function packTimeLines(time: PackTimeInfo): string[] {
  const lines = [`Оценка с коррекцией: ${formatHoursMinutes(time.correctedSeconds)}`];
  if (time.windowHours != null) {
    lines.push(`окно ${formatDecimal(time.windowHours)} ч, на столе ${time.chosenCount} из ${time.fullCount}`);
  }
  return lines;
}

/** Русское множественное число: forms — [1 объект, 3 объекта, 5 объектов]. */
export function pluralRu(count: number, [one, few, many]: readonly [string, string, string]): string {
  const mod100 = count % 100;
  const mod10 = count % 10;
  if (mod100 >= 11 && mod100 <= 14) return many;
  if (mod10 === 1) return one;
  if (mod10 >= 2 && mod10 <= 4) return few;
  return many;
}

/** «1 объект», «3 объекта», «37 объектов». */
export function objectsText(count: number): string {
  return `${count} ${pluralRu(count, ['объект', 'объекта', 'объектов'])}`;
}

const SHORT_DATE_TIME: Intl.DateTimeFormatOptions = {
  day: '2-digit',
  month: '2-digit',
  hour: '2-digit',
  minute: '2-digit',
};

/** «03.11, 14:05» — даты в списках задач и сроки хранения файлов. */
export function formatShortDateTime(iso: string): string {
  return new Date(iso).toLocaleString('ru-RU', SHORT_DATE_TIME);
}

/** Полная дата и время в локали браузера (админка, журнал). */
export function formatDateTime(iso: string): string {
  return new Date(iso).toLocaleString();
}
