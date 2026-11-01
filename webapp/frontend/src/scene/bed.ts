import type { PrinterProfile } from '../api/types';

export interface Rect {
  x0: number;
  y0: number;
  x1: number;
  y1: number;
}
export interface BedLabel {
  x: number;
  y: number;
  text: string;
}
export interface BedSpec {
  /** Контур пластины, мм, координаты стола (y — к задней стороне). */
  plate: [number, number][];
  /** printable_area профиля. */
  area: Rect;
  /** Область минус отступ от края — где реально может стоять объект. */
  usable: Rect;
  exclusions: Rect[];
  /** Подписи размеров и зон, z = 0. */
  labels: BedLabel[];
}

const DIM_OFFSET = 22; // мм от края пластины до размерной линии
const fmt = (v: number) => String(Math.round(v * 10) / 10);

export function bedSpec(profile: PrinterProfile): BedSpec {
  const [x0, y0, x1, y1] = profile.printableArea;
  const area = { x0, y0, x1, y1 };
  const m = profile.edgeMarginMm;
  const usable = { x0: x0 + m, y0: y0 + m, x1: x1 - m, y1: y1 - m };
  const plate: [number, number][] = profile.plateOutline?.length
    ? profile.plateOutline.map(([x, y]) => [x, y])
    : [
        [x0 - 3, y0 - 3],
        [x1 + 3, y0 - 3],
        [x1 + 3, y1 + 3],
        [x0 - 3, y1 + 3],
      ];
  const exclusions = profile.exclusionRectangles.map(([a, b, c, d]) => ({ x0: a, y0: b, x1: c, y1: d }));
  const minY = Math.min(...plate.map((p) => p[1]));
  const maxX = Math.max(...plate.map((p) => p[0]));
  const labels: BedLabel[] = [
    { x: (x0 + x1) / 2, y: minY - DIM_OFFSET, text: `${fmt(x1 - x0)} мм` },
    { x: maxX + DIM_OFFSET, y: (y0 + y1) / 2, text: `${fmt(y1 - y0)} мм` },
    ...(m > 0 ? [{ x: x0 + m + 22, y: y1 - m / 2, text: `отступ ${fmt(m)} мм` }] : []),
    ...exclusions.map((r) => ({ x: r.x1 + 20, y: (r.y0 + r.y1) / 2, text: `${fmt(r.x1 - r.x0)}×${fmt(r.y1 - r.y0)}` })),
  ];
  return { plate, area, usable, exclusions, labels };
}
