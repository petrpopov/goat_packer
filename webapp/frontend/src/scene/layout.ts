import type { PackLayout } from '../api/types';
import type { Rect } from './bed';

/** Копия на столе: центр габарита модели, мм, и угол против часовой. */
export interface Placement { x: number; y: number; angleDeg: number }

export interface SceneLayout {
  placements: Placement[];
  footprintLoops: [number, number][][] | null;
  tower: Rect | null;
}

/** layout-событие пакера → пропсы сцены. Порядок копий — порядок пакера (от центра). */
export function sceneLayout(layout: PackLayout | null | undefined): SceneLayout {
  if (!layout) return { placements: [], footprintLoops: null, tower: null };
  const [x0, y0, x1, y1] = layout.tower ?? [];
  return {
    placements: layout.placements.map(([x, y, angleDeg]) => ({ x, y, angleDeg })),
    footprintLoops: layout.footprint.length ? layout.footprint : null,
    tower: layout.tower ? { x0, y0, x1, y1 } : null,
  };
}
