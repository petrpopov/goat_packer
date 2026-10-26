/** Форматирование величин для интерфейса (числа — с точкой, единицы — на русском). */

import type { PackTimeInfo } from '../api/types';

export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return '—';
  if (bytes < 1024) return `${bytes} Б`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} КБ`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} МБ`;
}

/** Секунды → минуты; целые — без дробной части. */
export function formatMinutes(seconds: number): string {
  if (!Number.isFinite(seconds)) return '—';
  const minutes = seconds / 60;
  return Number.isInteger(minutes) ? String(minutes) : minutes.toFixed(1);
}

/** Граммы; целые — без дробной части. */
export function formatGrams(grams: number): string {
  if (!Number.isFinite(grams)) return '—';
  return Number.isInteger(grams) ? String(grams) : grams.toFixed(1);
}

/** Секунды → «Ч ч ММ мин» (минуты округляются, всегда две цифры). */
export function formatHoursMinutes(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return '—';
  const total = Math.round(seconds / 60);
  return `${Math.floor(total / 60)} ч ${String(total % 60).padStart(2, '0')} мин`;
}

/** Строки сводки времени упакованного стола: оценка с коррекцией и, с окном, «на столе K из M». */
export function packTimeLines(time: PackTimeInfo): string[] {
  const lines = [`Оценка с коррекцией: ${formatHoursMinutes(time.correctedSeconds)}`];
  if (time.windowHours != null) {
    const hours = Number.isInteger(time.windowHours) ? String(time.windowHours) : time.windowHours.toFixed(1);
    lines.push(`окно ${hours} ч, на столе ${time.chosenCount} из ${time.fullCount}`);
  }
  return lines;
}
