/** Форматирование величин для интерфейса (числа — с точкой, единицы — на русском). */

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
