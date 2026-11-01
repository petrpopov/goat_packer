/** Границы полей форм — те же, что проверяет бэкенд (PackParams/ScheduleParams/CustomBed). */
export interface Range {
  min: number;
  max: number;
}

export const LIMITS = {
  clearanceMm: { min: 0.05, max: 10 },
  resolutionMm: { min: 0.05, max: 1 },
  edgeMarginMm: { min: 0, max: 20 },
  windowHours: { min: 0.5, max: 240 },
  marginPct: { min: 0, max: 50 },
  changeSeconds: { min: 0, max: 600 },
  supportOverlapMm: { min: 0, max: 20 },
  scheduleMinutes: { min: 0, max: 600 },
} as const satisfies Record<string, Range>;

/** Подсказка о границах: «0.5–240 ч». */
export const rangeHint = ({ min, max }: Range, unit: string) => `${min}–${max} ${unit}`;

export const inRange = (value: string, { min, max }: Range, integer = false) => numberInRange(value, min, max, integer);

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

export type NozzleCleanMode = 'off' | 'global' | 'from-z';

export function buildNozzleClean(
  mode: NozzleCleanMode,
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

/**
 * «Перекрытие поддержек, мм»: пусто — без ограничений (поле не шлётся, CLI без флага),
 * иначе 0–20 мм, как на бэкенде. Бросает на невалидном значении.
 */
export function buildSupportOverlap(value: string): [string, string][] {
  if (value.trim() === '') return [];
  if (!inRange(value, LIMITS.supportOverlapMm))
    throw new Error(`Перекрытие поддержек: ${rangeHint(LIMITS.supportOverlapMm, 'мм')}`);
  return [['supportOverlapMm', String(Number(value))]];
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
    if (!inRange(state.windowHours, LIMITS.windowHours))
      throw new Error(`Окно печати: ${rangeHint(LIMITS.windowHours, 'ч')}`);
    if (!inRange(state.marginPct, LIMITS.marginPct)) throw new Error(`Запас: ${rangeHint(LIMITS.marginPct, '%')}`);
    fields.push(['timeWindowHours', String(Number(state.windowHours))]);
    fields.push(['timeMarginPct', String(Number(state.marginPct))]);
  }
  if (state.amsOn && amsAvailable) {
    fields.push(['amsCorrection', 'true']);
    if (state.changeSeconds.trim() !== '') {
      if (!inRange(state.changeSeconds, LIMITS.changeSeconds))
        throw new Error(`Смена прутка: ${rangeHint(LIMITS.changeSeconds, 'с')}`);
      fields.push(['changeSeconds', String(Number(state.changeSeconds))]);
    }
  }
  return fields;
}

/**
 * Поля времени для упаковки: без окна стол заполняется максимально и время не влияет на
 * результат, поэтому коррекция смены прутка шлётся только вместе с окном печати.
 */
export function packTimeFields(state: TimeFormState, amsAvailable: boolean): [string, string][] {
  return buildTimeFields({ ...state, amsOn: state.windowOn && state.amsOn }, amsAvailable);
}

/** Обратное buildNozzleClean: спецификация задачи → поля формы (задача, открытая по ссылке). */
export function parseNozzleClean(spec: string): { mode: NozzleCleanMode; fromZ: string; every: string } {
  const every = /every=([\d.]+)/.exec(spec)?.[1] ?? '';
  if (spec.startsWith('global')) return { mode: 'global', fromZ: '', every: every || '10' };
  const fromZ = /^from-z=([\d.]+)/.exec(spec)?.[1];
  if (fromZ) return { mode: 'from-z', fromZ, every: every || '1' };
  return { mode: 'off', fromZ: '', every: '' };
}
