/**
 * Каталог принтеров для выбора в PrinterPicker: группировка по производителю и поиск.
 * Чистые функции — без React, покрыты tests/printers.test.ts.
 */
import type { PrinterProfile, ProjectBed } from '../api/types';

/** Минимум полей профиля, который нужен группировке (PrinterProfile подходит целиком). */
export interface PrinterLike {
  name: string;
  vendor?: string;
}

export interface PrinterGroup<T> {
  vendor: string;
  items: T[];
}

const compare = (a: string, b: string) => a.localeCompare(b, 'ru', { sensitivity: 'base' });

/**
 * Производитель профиля. Пока /api/profiles не отдаёт `vendor` (ручные профили P1S/A2L/U1),
 * берём имя без последнего слова: «Bambu Lab P1S» → «Bambu Lab» — то же правило, что на бэкенде.
 */
export function vendorOf(profile: PrinterLike): string {
  const vendor = profile.vendor?.trim();
  if (vendor) return vendor;
  const words = profile.name.trim().split(/\s+/);
  return words.length > 1 ? words.slice(0, -1).join(' ') : (words[0] ?? '');
}

/** Производители A→Я, модели внутри A→Я (без учёта регистра и диакритики). */
export function groupPrinters<T extends PrinterLike>(list: readonly T[]): PrinterGroup<T>[] {
  const byVendor = new Map<string, T[]>();
  for (const item of list) {
    const vendor = vendorOf(item);
    const items = byVendor.get(vendor);
    if (items) items.push(item);
    else byVendor.set(vendor, [item]);
  }
  return [...byVendor.entries()]
    .sort(([a], [b]) => compare(a, b))
    .map(([vendor, items]) => ({ vendor, items: items.sort((a, b) => compare(a.name, b.name)) }));
}

/** Каждое слово запроса должно встретиться в «производитель модель»; пустой запрос — все. */
export function filterPrinters<T extends PrinterLike>(list: readonly T[], query: string): T[] {
  const tokens = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (tokens.length === 0) return [...list];
  return list.filter((item) => {
    const haystack = `${vendorOf(item)} ${item.name}`.toLowerCase();
    return tokens.every((token) => haystack.includes(token));
  });
}

/** «Моего принтера нет в списке»: размеры стола, мм (строки полей формы). */
export interface CustomBedInput {
  width: string;
  depth: string;
  height: string;
  margin: string;
}

export interface CustomBed {
  width: number;
  depth: number;
  height: number;
  margin: number;
}

export const CUSTOM_BED_MIN_MM = 50;
export const CUSTOM_BED_MAX_MM = 1000;
export const CUSTOM_MARGIN_MAX_MM = 30;

const inRange = (raw: string, min: number, max: number): number | null => {
  const trimmed = raw.trim();
  if (trimmed === '') return null;
  const value = Number(trimmed);
  return Number.isFinite(value) && value >= min && value <= max ? value : null;
};

/** Разбор полей своего стола с теми же границами, что CustomBed.validate на бэкенде; null — ошибка. */
export function parseCustomBed(input: CustomBedInput): CustomBed | null {
  const width = inRange(input.width, CUSTOM_BED_MIN_MM, CUSTOM_BED_MAX_MM);
  const depth = inRange(input.depth, CUSTOM_BED_MIN_MM, CUSTOM_BED_MAX_MM);
  const height = inRange(input.height, CUSTOM_BED_MIN_MM, CUSTOM_BED_MAX_MM);
  const margin = input.margin.trim() === '' ? 5 : inRange(input.margin, 0, CUSTOM_MARGIN_MAX_MM);
  if (width === null || depth === null || height === null || margin === null) return null;
  if (margin * 2 >= Math.min(width, depth)) return null;
  return { width, depth, height, margin };
}

/**
 * Профиль для сцены своего стола (как разовый CustomProfile бэкенда): прямоугольник от (0, 0) без
 * исключений; стол из проекта неизвестного принтера — с его началом координат и исключениями, пока
 * размеры совпадают с ним (пользователь их не правил).
 */
export function customPrinterProfile(bed: CustomBed, projectBed?: ProjectBed | null): PrinterProfile {
  const area = projectBed?.printableArea;
  const confirmed =
    area !== undefined &&
    area.length === 4 &&
    Math.abs(area[2] - area[0] - bed.width) < 0.01 &&
    Math.abs(area[3] - area[1] - bed.depth) < 0.01;
  const printableArea = confirmed ? [...area] : [0, 0, bed.width, bed.depth];
  return {
    id: 'custom',
    name: 'Свой принтер',
    vendor: 'Custom',
    slicerKind: 'orca',
    bedWidthMm: bed.width,
    bedDepthMm: bed.depth,
    edgeMarginMm: bed.margin,
    supportsNozzleClean: false,
    filamentChangeSeconds: null,
    printableArea,
    exclusionRectangles: confirmed ? projectBed!.exclusionRectangles.map((r) => [...r]) : [],
    plateOutline: null,
  };
}

/** Поля своего стола из стола проекта (неизвестный принтер 3MF): ширина/глубина/высота, отступ по умолчанию. */
export function customBedFromProject(projectBed: ProjectBed): CustomBedInput {
  const [x0, y0, x1, y1] = projectBed.printableArea;
  const round = (v: number) => String(Math.round(v * 100) / 100);
  return {
    width: round(x1 - x0),
    depth: round(y1 - y0),
    height: projectBed.printableHeight != null ? round(projectBed.printableHeight) : '',
    margin: '5',
  };
}

/** STL — по имени файла (так же решает бэкенд). */
export const isStlName = (name: string) => name.trim().toLowerCase().endsWith('.stl');
