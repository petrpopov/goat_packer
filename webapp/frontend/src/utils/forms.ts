export function numberInRange(value: string, min: number, max: number, integer = false): boolean {
  if (value.trim() === '') return false;
  const n = Number(value);
  return Number.isFinite(n) && n >= min && n <= max && (!integer || Number.isInteger(n));
}

export function clockSeconds(value: string): number | null {
  const match = /^(\d{1,2}):([0-5]\d)$/.exec(value.trim());
  if (!match || Number(match[1]) > 47) return null;
  return Number(match[1]) * 3600 + Number(match[2]) * 60;
}

export function validTimeWindow(day: string, night: string, end: string): boolean {
  const d = clockSeconds(day),
    n = clockSeconds(night),
    e = clockSeconds(end);
  return d !== null && n !== null && e !== null && d < n && n < e;
}

export function buildNozzleClean(
  mode: 'off' | 'global' | 'from-z',
  fromZ: string,
  globalEvery: string,
  fromZEvery: string
): string {
  if (mode === 'off') return '';
  const every = mode === 'global' ? globalEvery : fromZEvery;
  if (!numberInRange(every, 1, Number.MAX_SAFE_INTEGER, true))
    throw new Error('Период очистки должен быть целым числом ≥ 1');
  if (mode === 'from-z' && !numberInRange(fromZ, Number.MIN_VALUE, 100000))
    throw new Error('Высота очистки должна быть положительным числом');
  // `global` without every means 10 in the CLI, not 1. Always send the actual UI value.
  return `${mode === 'global' ? 'global' : `from-z=${Number(fromZ)}`},every=${Number(every)}`;
}

export interface TimeFormState {
  /** «Уложиться в окно печати». */
  windowOn: boolean;
  windowHours: string;
  marginPct: string;
  /** «Коррекция времени смены прутка (AMS)». */
  amsOn: boolean;
  changeSeconds: string;
}

/**
 * Поля формы времени печати — только заданные пользователем (иначе действуют дефолты CLI).
 * Длительность смены уходит только с включённой коррекцией: в CLI она сама включает коррекцию.
 * Пустая длительность = значение профиля. Бросает на невалидных значениях (диапазоны — как на бэкенде).
 */
export function buildTimeFields(state: TimeFormState, amsAvailable: boolean): [string, string][] {
  const fields: [string, string][] = [];
  if (state.windowOn) {
    if (!numberInRange(state.windowHours, 0.5, 240)) throw new Error('Окно печати: 0.5–240 ч');
    if (!numberInRange(state.marginPct, 0, 50)) throw new Error('Запас: 0–50 %');
    fields.push(['timeWindowHours', String(Number(state.windowHours))]);
    fields.push(['timeMarginPct', String(Number(state.marginPct))]);
  }
  if (state.amsOn && amsAvailable) {
    fields.push(['amsCorrection', 'true']);
    if (state.changeSeconds.trim() !== '') {
      if (!numberInRange(state.changeSeconds, 0, 600)) throw new Error('Смена прутка: 0–600 с');
      fields.push(['changeSeconds', String(Number(state.changeSeconds))]);
    }
  }
  return fields;
}
