/**
 * Проекция модели на стол растром (подложка сцены до раскладки). Треугольники заливаются здесь
 * построчно, а не canvas-путём: один путь из сотен тысяч подпутей с заливкой nonzero Chrome
 * заливал ~100 с в основном потоке (Firefox — доли секунды), страница висела после выбора файла.
 */
export interface ProjectionMask {
  /** 1 — пиксель внутри проекции; строки сверху вниз (y растёт вниз, как в canvas). */
  mask: Uint8Array;
  width: number;
  height: number;
  /** Пикселей на мм. */
  res: number;
  /** Поле вокруг габарита, мм (под обводку). */
  pad: number;
  /** Габарит проекции, мм. */
  minX: number;
  minY: number;
  maxX: number;
  maxY: number;
}

/** positions — по 9 чисел на треугольник (как ModelMesh.positions); z не используется. */
export function projectTriangles(positions: Float32Array, triangles: number, res = 4, pad = 3): ProjectionMask {
  let minX = Infinity,
    minY = Infinity,
    maxX = -Infinity,
    maxY = -Infinity;
  for (let t = 0; t < triangles; t++)
    for (let k = 0; k < 3; k++) {
      const x = positions[t * 9 + k * 3],
        y = positions[t * 9 + k * 3 + 1];
      if (x < minX) minX = x;
      if (x > maxX) maxX = x;
      if (y < minY) minY = y;
      if (y > maxY) maxY = y;
    }
  if (!Number.isFinite(minX)) minX = minY = maxX = maxY = 0;
  const width = Math.max(2, Math.ceil((maxX - minX + pad * 2) * res));
  const height = Math.max(2, Math.ceil((maxY - minY + pad * 2) * res));
  const mask = new Uint8Array(width * height);
  const px = (x: number) => (x - minX + pad) * res;
  const py = (y: number) => height - (y - minY + pad) * res;
  for (let t = 0; t < triangles; t++) {
    const o = t * 9;
    let ax = px(positions[o]),
      ay = py(positions[o + 1]);
    let bx = px(positions[o + 3]),
      by = py(positions[o + 4]);
    const cx = px(positions[o + 6]),
      cy = py(positions[o + 7]);
    let area = (bx - ax) * (cy - ay) - (by - ay) * (cx - ax);
    if (area === 0) continue; // вертикальная грань: в проекции — линия
    if (area < 0) {
      // Обход к одному знаку: нижние грани в проекции идут в другую сторону.
      [ax, bx] = [bx, ax];
      [ay, by] = [by, ay];
      area = -area;
    }
    // Центры пикселей внутри треугольника (границы включительно — соседние грани без щелей).
    const i0 = Math.max(0, Math.floor(Math.min(ax, bx, cx))),
      i1 = Math.min(width - 1, Math.ceil(Math.max(ax, bx, cx)));
    const j0 = Math.max(0, Math.floor(Math.min(ay, by, cy))),
      j1 = Math.min(height - 1, Math.ceil(Math.max(ay, by, cy)));
    for (let j = j0; j <= j1; j++) {
      const y = j + 0.5;
      for (let i = i0; i <= i1; i++) {
        const x = i + 0.5;
        if (
          (bx - ax) * (y - ay) - (by - ay) * (x - ax) >= 0 &&
          (cx - bx) * (y - by) - (cy - by) * (x - bx) >= 0 &&
          (ax - cx) * (y - cy) - (ay - cy) * (x - cx) >= 0
        )
          mask[j * width + i] = 1;
      }
    }
  }
  return { mask, width, height, res, pad, minX, minY, maxX, maxY };
}
