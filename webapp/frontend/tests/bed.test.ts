import { describe, expect, test } from 'bun:test';
import type { PrinterProfile } from '../src/api/types';
import { bedSpec } from '../src/scene/bed';

const base: PrinterProfile = {
  id: 'x', name: 'X', slicerKind: 'bambu', bedWidthMm: 256, bedDepthMm: 256, edgeMarginMm: 0,
  supportsNozzleClean: false, printableArea: [0, 0, 256, 256], exclusionRectangles: [], plateOutline: null,
};

describe('bedSpec', () => {
  test('P1S: dimensions along two sides and the 18×28 corner', () => {
    const bed = bedSpec({ ...base, exclusionRectangles: [[0, 0, 18, 28]], plateOutline: [[0, 0], [258, 0], [258, 258], [0, 258]] });
    expect(bed.area).toEqual({ x0: 0, y0: 0, x1: 256, y1: 256 });
    expect(bed.usable).toEqual(bed.area);
    expect(bed.labels.map((l) => l.text)).toEqual(['256 мм', '256 мм', '18×28']);
    expect(bed.exclusions).toEqual([{ x0: 0, y0: 0, x1: 18, y1: 28 }]);
  });
  test('U1: edge margin shrinks the usable area and gets a label', () => {
    const bed = bedSpec({ ...base, edgeMarginMm: 4, printableArea: [0.5, 1, 270.5, 271] });
    expect(bed.usable).toEqual({ x0: 4.5, y0: 5, x1: 266.5, y1: 267 });
    expect(bed.labels.map((l) => l.text)).toEqual(['270 мм', '270 мм', 'отступ 4 мм']);
  });
  test('no outline: plate is the printable area grown by 3 mm', () => {
    const bed = bedSpec(base);
    expect(bed.plate).toEqual([[-3, -3], [259, -3], [259, 259], [-3, 259]]);
  });
  test('labels sit outside the plate: front edge below min y, side edge right of max x', () => {
    const bed = bedSpec(base);
    expect(bed.labels[0].y).toBeLessThan(-3);
    expect(bed.labels[1].x).toBeGreaterThan(259);
  });
});
