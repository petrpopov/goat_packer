/** Форма упаковки без React: значения полей, их проверка и запрос на сервер. */

import type { PackParamsView } from '@/api/types';
import {
  buildNozzleClean,
  buildSupportOverlap,
  inRange,
  LIMITS,
  packTimeFields,
  parseNozzleClean,
  type NozzleCleanMode,
} from '@/utils/forms';
import type { CustomBed, CustomBedInput } from '@/utils/printers';

export type PackLayoutMode = 'search' | 'grid';

/** Поля формы упаковки строками — как их ввёл пользователь. */
export interface PackFormValues {
  clearance: string;
  resolution: string;
  layout: PackLayoutMode;
  cleanMode: NozzleCleanMode;
  globalEvery: string;
  fromZ: string;
  fromZEvery: string;
  /** Пусто — дефолт профиля. */
  edgeMargin: string;
  /** Ширина черновой башни; пусто — дефолт пакера ([DEFAULT_TOWER_WIDTH_MM]). */
  towerWidth: string;
  /** Сразу нарезать весь стол; иначе — по кнопке «Проверить нарезкой» после упаковки (минуты ожидания). */
  sliceNow: boolean;
  windowOn: boolean;
  windowHours: string;
  marginPct: string;
  /** Пусто — без ограничений. */
  supportOverlap: string;
  /** Слайсер STL (bambu|orca); пусто — умолчание принтера. */
  slicer: string;
  /** Свой стол («Моего принтера нет в списке» или 3MF неизвестного принтера). */
  bed: CustomBedInput;
}

export const DEFAULT_CUSTOM_MARGIN_MM = '5';
/** Ширина башни, которую пакер ставит без --tower-width (поле пустое — не шлётся). */
export const DEFAULT_TOWER_WIDTH_MM = 20;
export const EMPTY_BED: CustomBedInput = { width: '', depth: '', height: '', margin: DEFAULT_CUSTOM_MARGIN_MM };

export const DEFAULT_PACK_FORM: PackFormValues = {
  clearance: '0.7',
  resolution: '0.25',
  layout: 'search',
  cleanMode: 'off',
  globalEvery: '10',
  fromZ: '',
  fromZEvery: '1',
  edgeMargin: '',
  towerWidth: '',
  sliceNow: false,
  windowOn: false,
  windowHours: '12',
  marginPct: '10',
  supportOverlap: '',
  slicer: '',
  bed: EMPTY_BED,
};

const text = (value: number | null) => (value == null ? '' : String(value));

/** Параметры задачи, открытой по ссылке, → поля формы (только для чтения; «Упаковать заново» их берёт). */
export function valuesFromParams(params: PackParamsView, current: PackFormValues): PackFormValues {
  const clean = parseNozzleClean(params.nozzleClean);
  return {
    ...current,
    clearance: String(params.clearance),
    resolution: String(params.resolution),
    layout: params.layout === 'grid' ? 'grid' : 'search',
    cleanMode: clean.mode,
    globalEvery: clean.mode === 'global' ? clean.every : current.globalEvery,
    fromZ: clean.mode === 'from-z' ? clean.fromZ : current.fromZ,
    fromZEvery: clean.mode === 'from-z' ? clean.every : current.fromZEvery,
    edgeMargin: text(params.edgeMargin),
    towerWidth: text(params.towerWidth ?? null),
    sliceNow: params.finalSlice,
    windowOn: params.timeWindowHours != null,
    windowHours: params.timeWindowHours != null ? String(params.timeWindowHours) : current.windowHours,
    marginPct: params.timeMarginPct != null ? String(params.timeMarginPct) : current.marginPct,
    supportOverlap: text(params.supportOverlapMm),
    slicer: params.slicer ?? '',
    bed: params.customBed
      ? {
          width: String(params.customBed.width),
          depth: String(params.customBed.depth),
          height: String(params.customBed.height),
          margin: String(params.customBed.margin),
        }
      : current.bed,
  };
}

type Fields = [string, string][];

/** Пусто (дефолт пакера) или число в границах. */
export const towerWidthValid = (value: string) => value.trim() === '' || inRange(value, LIMITS.towerWidthMm);

/** Результат построителя полей: поля или признак ошибки (подсветка и блокировка «Упаковать»). */
function attempt<T>(build: () => T, empty: T): { value: T; invalid: boolean } {
  try {
    return { value: build(), invalid: false };
  } catch {
    return { value: empty, invalid: true };
  }
}

export interface PackRequestContext {
  /** У принтера есть код очистки сопла. */
  nozzleCleanAvailable: boolean;
  ams: { on: boolean; seconds: string; available: boolean };
}

/** Проверенные части запроса и какие из них с ошибкой. */
export interface PackRequestParts {
  nozzleSpec: string;
  nozzleCleanInvalid: boolean;
  timeFields: Fields;
  timeInvalid: boolean;
  supportFields: Fields;
  supportInvalid: boolean;
  /** Зазор, растр, отступ и ширина башни в границах. */
  numbersValid: boolean;
  valid: boolean;
}

export function packRequestParts(values: PackFormValues, context: PackRequestContext): PackRequestParts {
  const cleanMode = context.nozzleCleanAvailable ? values.cleanMode : 'off';
  const nozzle = attempt(() => buildNozzleClean(cleanMode, values.fromZ, values.globalEvery, values.fromZEvery), '');
  // Коррекция смены прутка влияет только на окно печати: без окна она не шлётся.
  const time = attempt<Fields>(
    () =>
      packTimeFields(
        {
          windowOn: values.windowOn,
          windowHours: values.windowHours,
          marginPct: values.marginPct,
          amsOn: context.ams.on,
          changeSeconds: context.ams.seconds,
        },
        context.ams.available
      ),
    []
  );
  const support = attempt<Fields>(() => buildSupportOverlap(values.supportOverlap), []);
  const numbersValid =
    inRange(values.clearance, LIMITS.clearanceMm) &&
    inRange(values.resolution, LIMITS.resolutionMm) &&
    (values.edgeMargin.trim() === '' || inRange(values.edgeMargin, LIMITS.edgeMarginMm)) &&
    towerWidthValid(values.towerWidth);
  return {
    nozzleSpec: nozzle.value,
    nozzleCleanInvalid: nozzle.invalid,
    timeFields: time.value,
    timeInvalid: time.invalid,
    supportFields: support.value,
    supportInvalid: support.invalid,
    numbersValid,
    valid: numbersValid && !nozzle.invalid && !time.invalid && !support.invalid,
  };
}

/** Принтер запроса: профиль каталога или свой стол. */
export type PackTarget = { kind: 'profile'; profileId: string } | { kind: 'custom'; bed: CustomBed };

/** multipart-запрос POST /pack/jobs (сервер разбирает его в JSON в api/services). */
export function packFormData(
  file: File,
  values: PackFormValues,
  target: PackTarget,
  parts: PackRequestParts,
  /** Слайсер STL-задачи (effectiveSlicer); null — 3MF, слайсер задаёт проект. */
  slicer: string | null = null
): FormData {
  const form = new FormData();
  form.append('file', file);
  if (target.kind === 'custom') {
    form.append('customWidth', String(target.bed.width));
    form.append('customDepth', String(target.bed.depth));
    form.append('customHeight', String(target.bed.height));
    form.append('customMargin', String(target.bed.margin));
  } else {
    form.append('profileId', target.profileId);
  }
  form.append('clearance', values.clearance);
  form.append('resolution', values.resolution);
  // attempts не шлём: действует дефолт бэкенда (= CLI). Ползунок усилия убран — с поддержками
  // поиск всегда перебирает фиксированные 64 варианта, число попыток ни на что не влияло.
  form.append('nozzleClean', parts.nozzleSpec);
  form.append('layout', values.layout);
  // Пустой отступ = дефолт профиля (у Snapmaker U1 — 4 мм против качания головки). Свой стол
  // несёт отступ в customMargin, второй (из «Дополнительно», там скрыт) не шлём.
  if (target.kind !== 'custom' && values.edgeMargin.trim() !== '') form.append('edgeMargin', values.edgeMargin.trim());
  // Пустая ширина башни = дефолт пакера (20 мм): поле не шлётся.
  if (values.towerWidth.trim() !== '') form.append('towerWidth', values.towerWidth.trim());
  // Окно и коррекция — только если заданы: иначе действуют дефолты CLI.
  for (const [name, value] of parts.timeFields) form.append(name, value);
  // Пустое перекрытие = без ограничений: поле не шлётся, флаг CLI не передаётся.
  for (const [name, value] of parts.supportFields) form.append(name, value);
  // Окно печати меряется нарезками — с ним стол режется всегда.
  form.append('finalSlice', String(values.sliceNow || values.windowOn));
  if (slicer) form.append('slicer', slicer);
  return form;
}
