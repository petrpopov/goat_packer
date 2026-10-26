/**
 * Калькулятор теста смены прутка AMS (спека 2026-10-26, раздел 5) и хранение результата в
 * localStorage. Чистые функции: страница, история задач и поле «Смена прутка, с» пользуются
 * одним и тем же расчётом и одним ключом хранения.
 */

/** Значения calibration.json, нужные калькулятору (секунды и число смен в B). */
export interface CalibrationValues {
  estimateASeconds: number;
  estimateBSeconds: number;
  changes: number;
  slicerChangeSeconds: number;
}

/** Итог задачи калибровки → значения калькулятора (число смен — фактически нарезанное). */
export function calibrationValues(info: {
  estimateASeconds: number;
  estimateBSeconds: number;
  changesSliced: number;
  slicerChangeSeconds: number;
}): CalibrationValues {
  return {
    estimateASeconds: info.estimateASeconds,
    estimateBSeconds: info.estimateBSeconds,
    changes: info.changesSliced,
    slicerChangeSeconds: info.slicerChangeSeconds,
  };
}

/** Четыре записанных времени печати, «ЧЧ:ММ». */
export interface CalibrationClocks {
  aStart: string;
  aFinish: string;
  bStart: string;
  bFinish: string;
}

export type CalibrationOutcome =
  { ok: true; extraSeconds: number; realSeconds: number; warning: string | null } | { ok: false; error: string };

/** «Лишнее» на смену вне 0..200 с почти наверняка значит ошибку в записанных временах. */
export const EXTRA_MIN_SECONDS = 0;
export const EXTRA_MAX_SECONDS = 200;
export const SUSPICIOUS_TIME_WARNING = 'похоже, время записано неверно';

const CLOCK = /^(\d{1,2}):([0-5]\d)$/;

/** «ЧЧ:ММ» (часы 0–23) → минуты от полуночи; null — не время. */
export function parseClockMinutes(value: string): number | null {
  const match = value.trim().match(CLOCK);
  if (!match) return null;
  const hours = Number(match[1]);
  if (hours > 23) return null;
  return hours * 60 + Number(match[2]);
}

/** Длительность печати, с; конец раньше начала — печать закончилась после полуночи (+24 ч). */
export function printDurationSeconds(start: string, finish: string): number | null {
  const s = parseClockMinutes(start);
  const f = parseClockMinutes(finish);
  if (s === null || f === null) return null;
  return ((f - s + 24 * 60) % (24 * 60)) * 60;
}

/**
 * extra = ((факт_B − оценка_B) − (факт_A − оценка_A)) / changes; real = extra + slicer_change_s,
 * округление до секунды. Подозрительное extra не скрывается — показывается с предупреждением.
 */
export function computeChangeSeconds(clocks: CalibrationClocks, values: CalibrationValues): CalibrationOutcome {
  const factA = printDurationSeconds(clocks.aStart, clocks.aFinish);
  const factB = printDurationSeconds(clocks.bStart, clocks.bFinish);
  if (factA === null || factB === null) return { ok: false, error: 'Введите все четыре времени в формате ЧЧ:ММ' };
  const { estimateASeconds, estimateBSeconds, changes, slicerChangeSeconds } = values;
  if (![estimateASeconds, estimateBSeconds, slicerChangeSeconds].every(Number.isFinite))
    return { ok: false, error: 'Укажите оценки слайсера и время смены в слайсере' };
  if (!Number.isInteger(changes) || changes < 1) return { ok: false, error: 'Число смен должно быть целым ≥ 1' };
  const extra = (factB - estimateBSeconds - (factA - estimateASeconds)) / changes;
  const realSeconds = Math.round(extra + slicerChangeSeconds);
  const warning = extra < EXTRA_MIN_SECONDS || extra > EXTRA_MAX_SECONDS ? SUSPICIOUS_TIME_WARNING : null;
  return { ok: true, extraSeconds: Math.round(extra), realSeconds, warning };
}

/** Вставленный текст calibration.json → значения калькулятора; null — не тот файл. */
export function parseCalibrationJson(text: string): CalibrationValues | null {
  let raw: unknown;
  try {
    raw = JSON.parse(text);
  } catch {
    return null;
  }
  if (typeof raw !== 'object' || raw === null) return null;
  const record = raw as Record<string, unknown>;
  const num = (key: string) => (typeof record[key] === 'number' ? (record[key] as number) : NaN);
  const values: CalibrationValues = {
    estimateASeconds: num('estimate_a_s'),
    estimateBSeconds: num('estimate_b_s'),
    changes: Number.isFinite(num('changes_sliced')) ? num('changes_sliced') : num('changes_expected'),
    slicerChangeSeconds: num('slicer_change_s'),
  };
  return Object.values(values).every(Number.isFinite) ? values : null;
}

/** Минимальный интерфейс хранилища (localStorage или заглушка в тестах). */
export interface KeyValueStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

/** Ключ localStorage: { [profileId]: секунды } — коррекция, сохранённая кнопкой калькулятора. */
export const CHANGE_SECONDS_KEY = 'goat-packer-ams-change-seconds';
/** Ключ localStorage: { [profileId]: CalibrationValues } — последний тест, для возврата позже. */
export const CALIBRATION_VALUES_KEY = 'goat-packer-ams-calibration';

function browserStorage(): KeyValueStorage | null {
  try {
    return typeof localStorage === 'undefined' ? null : localStorage;
  } catch {
    return null;
  }
}

function readMap(key: string, storage: KeyValueStorage | null): Record<string, unknown> {
  if (!storage) return {};
  try {
    const parsed: unknown = JSON.parse(storage.getItem(key) ?? '{}');
    return typeof parsed === 'object' && parsed !== null && !Array.isArray(parsed)
      ? (parsed as Record<string, unknown>)
      : {};
  } catch {
    return {};
  }
}

function writeEntry(key: string, profileId: string, value: unknown, storage: KeyValueStorage | null): void {
  if (!storage) return;
  try {
    storage.setItem(key, JSON.stringify({ ...readMap(key, storage), [profileId]: value }));
  } catch {
    /* приватный режим / переполнение — значение просто не сохранится */
  }
}

/** Все сохранённые коррекции { profileId: секунды }; битые записи отбрасываются. */
export function readStoredChangeSeconds(storage: KeyValueStorage | null = browserStorage()): Record<string, number> {
  const result: Record<string, number> = {};
  for (const [profileId, value] of Object.entries(readMap(CHANGE_SECONDS_KEY, storage))) {
    if (typeof value === 'number' && Number.isFinite(value) && value >= 0 && value <= 600) result[profileId] = value;
  }
  return result;
}

export function storeChangeSeconds(
  profileId: string,
  seconds: number,
  storage: KeyValueStorage | null = browserStorage()
): void {
  writeEntry(CHANGE_SECONDS_KEY, profileId, seconds, storage);
}

export function readStoredCalibration(
  profileId: string,
  storage: KeyValueStorage | null = browserStorage()
): CalibrationValues | null {
  const value = readMap(CALIBRATION_VALUES_KEY, storage)[profileId];
  if (typeof value !== 'object' || value === null) return null;
  const v = value as Record<string, unknown>;
  const values = {
    estimateASeconds: Number(v.estimateASeconds),
    estimateBSeconds: Number(v.estimateBSeconds),
    changes: Number(v.changes),
    slicerChangeSeconds: Number(v.slicerChangeSeconds),
  };
  return Object.values(values).every(Number.isFinite) ? values : null;
}

export function storeCalibration(
  profileId: string,
  values: CalibrationValues,
  storage: KeyValueStorage | null = browserStorage()
): void {
  writeEntry(CALIBRATION_VALUES_KEY, profileId, values, storage);
}

/**
 * Значение поля «Смена прутка, с» по умолчанию: сохранённая калибровка этого принтера важнее
 * значения профиля; без значения профиля (тулчейнджер) коррекция недоступна — пусто.
 */
export function defaultChangeSeconds(profileSeconds: number | null, storedSeconds: number | undefined): string {
  if (profileSeconds === null) return '';
  return String(storedSeconds ?? profileSeconds);
}
