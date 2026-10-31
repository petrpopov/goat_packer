import type { PackLayout } from '../api/types';
import type { Rect } from './bed';

/** Копия на столе: центр габарита модели, мм, и угол против часовой. */
export interface Placement { x: number; y: number; angleDeg: number }

export interface SceneLayout {
  placements: Placement[];
  footprintLoops: [number, number][][] | null;
  tower: Rect | null;
  /** Рабочая область пакера (с отступом от края, заданным пользователем); null — раскладки нет. */
  bed: Rect | null;
}

/** layout-событие пакера → пропсы сцены. Порядок копий — порядок пакера (от центра). */
export function sceneLayout(layout: PackLayout | null | undefined): SceneLayout {
  if (!layout) return { placements: [], footprintLoops: null, tower: null, bed: null };
  const [x0, y0, x1, y1] = layout.tower ?? [];
  const [bx0, by0, bx1, by1] = layout.bed;
  return {
    placements: layout.placements.map(([x, y, angleDeg]) => ({ x, y, angleDeg })),
    footprintLoops: layout.footprint.length ? layout.footprint : null,
    tower: layout.tower ? { x0, y0, x1, y1 } : null,
    bed: layout.bed.length === 4 ? { x0: bx0, y0: by0, x1: bx1, y1: by1 } : null,
  };
}
