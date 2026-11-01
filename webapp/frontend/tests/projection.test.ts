import { describe, expect, test } from 'bun:test';
import { projectTriangles } from '../src/scene/projection';

describe('projectTriangles', () => {
  test('квадрат 10×10 из двух граней разного обхода заливается целиком, без щели по диагонали', () => {
    // Первая грань против часовой, вторая — по часовой (как нижние грани в проекции).
    const positions = Float32Array.from([0, 0, 0, 10, 0, 0, 10, 10, 0, 0, 0, 0, 0, 10, 0, 10, 10, 0]);
    const p = projectTriangles(positions, 2, 4, 3);
    expect([p.width, p.height]).toEqual([64, 64]);
    let filled = 0;
    for (const v of p.mask) filled += v;
    expect(filled).toBe(40 * 40);
    // Пиксель внутри и пиксель в поле: строки сверху вниз, поле 3 мм = 12 px.
    expect(p.mask[20 * 64 + 20]).toBe(1);
    expect(p.mask[5 * 64 + 5]).toBe(0);
  });

  test('вертикальная грань (нулевая площадь в проекции) ничего не заливает', () => {
    const p = projectTriangles(Float32Array.from([0, 0, 0, 10, 0, 0, 10, 0, 5]), 1);
    expect(p.mask.every((v) => v === 0)).toBe(true);
  });
});
