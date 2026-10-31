/**
 * Каталог принтеров для выбора в PrinterPicker: группировка по производителю и поиск.
 * Чистые функции — без React, покрыты tests/printers.test.ts.
 */

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
  return words.length > 1 ? words.slice(0, -1).join(' ') : words[0] ?? '';
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
