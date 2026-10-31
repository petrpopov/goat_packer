import { describe, expect, test } from 'bun:test';
import type { PackLayout } from '../src/api/types';
import { sceneLayout } from '../src/scene/layout';

const layout: PackLayout = {
  source: 'repack', objects: 2, bed: [0, 0, 256, 256], keepouts: [[0, 0, 22, 32]], tower: [214, 214, 246, 246],
  footprint: [[[-5, -5], [5, -5], [5, 5], [-5, -5]]], placements: [[20, 20, 0], [40, 20, 90]], epsilon_mm: 0,
};

describe('sceneLayout', () => {
  test('placements become scene placements in packer order (centre-out)', () => {
    expect(sceneLayout(layout).placements).toEqual([
      { x: 20, y: 20, angleDeg: 0 },
      { x: 40, y: 20, angleDeg: 90 },
    ]);
    expect(sceneLayout(layout).footprintLoops).toEqual(layout.footprint);
  });
  test('tower becomes a rect; missing layout gives an empty scene layout', () => {
    expect(sceneLayout(layout).tower).toEqual({ x0: 214, y0: 214, x1: 246, y1: 246 });
    expect(sceneLayout({ ...layout, tower: null }).tower).toBeNull();
    expect(sceneLayout({ ...layout, footprint: [] }).footprintLoops).toBeNull();
    expect(sceneLayout(null)).toEqual({ placements: [], footprintLoops: null, tower: null, bed: null });
    expect(sceneLayout(undefined)).toEqual({ placements: [], footprintLoops: null, tower: null, bed: null });
  });
  test('usable bed comes from the packer (user edge margin), not the profile', () => {
    expect(sceneLayout({ ...layout, bed: [10, 10, 246, 246] }).bed).toEqual({ x0: 10, y0: 10, x1: 246, y1: 246 });
    expect(sceneLayout({ ...layout, bed: [] }).bed).toBeNull();
  });
});
