import * as THREE from 'three';
import type { ProjectionMask } from './projection';
import { cssValue } from './three';

/** Источник подложки: контуры пакера (even-odd, их немного) или растр проекции меша. */
export type FootprintSource =
  { kind: 'loops'; polygons: [number, number][][] } | { kind: 'mask'; projection: ProjectionMask };

/** Растр контуров пакера: пикселей на мм и поле вокруг, мм. */
const LOOPS_PX_PER_MM = 4;
const LOOPS_PAD_MM = 3;
/** Обводка подложки, мм (рисуется 16 сдвигами силуэта по окружности этого радиуса). */
const RING_RADIUS_MM = 0.8;
const RING_STEPS = 16;
/** Заливка подложки — доля непрозрачности цвета обводки. */
const FILL_ALPHA = 0.28;
const FOOTPRINT_TOKEN = '--accent-2';

interface Silhouette {
  canvas: HTMLCanvasElement;
  res: number;
  minX: number;
  minY: number;
  maxX: number;
  maxY: number;
}

function maskSilhouette(p: ProjectionMask): Silhouette {
  const canvas = document.createElement('canvas');
  canvas.width = p.width;
  canvas.height = p.height;
  const image = new ImageData(p.width, p.height);
  for (let i = 0; i < p.mask.length; i++) if (p.mask[i]) image.data.set([255, 255, 255, 255], i * 4);
  canvas.getContext('2d')!.putImageData(image, 0, 0);
  return { canvas, res: p.res, minX: p.minX, minY: p.minY, maxX: p.maxX, maxY: p.maxY };
}

function loopsSilhouette(polygons: [number, number][][]): Silhouette {
  let minX = Infinity,
    minY = Infinity,
    maxX = -Infinity,
    maxY = -Infinity;
  for (const poly of polygons)
    for (const [x, y] of poly) {
      minX = Math.min(minX, x);
      maxX = Math.max(maxX, x);
      minY = Math.min(minY, y);
      maxY = Math.max(maxY, y);
    }
  if (!Number.isFinite(minX)) minX = minY = maxX = maxY = 0;
  const res = LOOPS_PX_PER_MM;
  const canvas = document.createElement('canvas');
  canvas.width = Math.max(2, Math.ceil((maxX - minX + LOOPS_PAD_MM * 2) * res));
  canvas.height = Math.max(2, Math.ceil((maxY - minY + LOOPS_PAD_MM * 2) * res));
  const toX = (x: number) => (x - minX + LOOPS_PAD_MM) * res;
  const toY = (y: number) => canvas.height - (y - minY + LOOPS_PAD_MM) * res;
  const context = canvas.getContext('2d')!;
  context.fillStyle = '#fff';
  context.beginPath();
  for (const poly of polygons)
    poly.forEach(([x, y], i) => (i ? context.lineTo(toX(x), toY(y)) : context.moveTo(toX(x), toY(y))));
  context.fill('evenodd');
  return { canvas, res, minX, minY, maxX, maxY };
}

/** Силуэт, залитый цветом (source-in). */
function tinted(silhouette: HTMLCanvasElement, color: string): HTMLCanvasElement {
  const canvas = document.createElement('canvas');
  canvas.width = silhouette.width;
  canvas.height = silhouette.height;
  const context = canvas.getContext('2d')!;
  context.drawImage(silhouette, 0, 0);
  context.globalCompositeOperation = 'source-in';
  context.fillStyle = color;
  context.fillRect(0, 0, canvas.width, canvas.height);
  return canvas;
}

/** Подложка «проекция + контур»: заливка FILL_ALPHA и обводка RING_RADIUS_MM цветом --accent-2. */
export function footprintTexture(source: FootprintSource): {
  texture: THREE.CanvasTexture;
  size: [number, number];
  center: [number, number];
} {
  const {
    canvas: silhouette,
    res,
    minX,
    minY,
    maxX,
    maxY,
  } = source.kind === 'mask' ? maskSilhouette(source.projection) : loopsSilhouette(source.polygons);
  const { width: w, height: h } = silhouette;
  const out = document.createElement('canvas');
  out.width = w;
  out.height = h;
  const context = out.getContext('2d')!;
  const ring = tinted(silhouette, cssValue(FOOTPRINT_TOKEN));
  const radius = RING_RADIUS_MM * res;
  for (let a = 0; a < RING_STEPS; a++) {
    const angle = (a / RING_STEPS) * 2 * Math.PI;
    context.drawImage(ring, Math.cos(angle) * radius, Math.sin(angle) * radius);
  }
  context.globalCompositeOperation = 'destination-out';
  context.drawImage(silhouette, 0, 0);
  context.globalCompositeOperation = 'source-over';
  context.globalAlpha = FILL_ALPHA;
  context.drawImage(ring, 0, 0);
  context.globalAlpha = 1;
  const texture = new THREE.CanvasTexture(out);
  texture.colorSpace = THREE.SRGBColorSpace;
  return { texture, size: [w / res, h / res], center: [(minX + maxX) / 2, (minY + maxY) / 2] };
}
