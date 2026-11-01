/** Мелкие помощники three.js для сцены стола: цвета темы, контуры, освобождение GPU-ресурсов. */

import * as THREE from 'three';
import type { Rect } from './bed';

/** Цвет, если CSS-переменная не задана. */
const FALLBACK_COLOR = '#888';

export const cssValue = (name: string) => getComputedStyle(document.documentElement).getPropertyValue(name).trim();

/** oklch/hex CSS-переменная → THREE.Color (через canvas: THREE не понимает oklch). */
export function cssColor(name: string): THREE.Color {
  const canvas = document.createElement('canvas');
  canvas.width = canvas.height = 1;
  const context = canvas.getContext('2d', { willReadFrequently: true })!;
  context.fillStyle = cssValue(name) || FALLBACK_COLOR;
  context.fillRect(0, 0, 1, 1);
  const [r, g, b] = context.getImageData(0, 0, 1, 1).data;
  return new THREE.Color(r / 255, g / 255, b / 255).convertSRGBToLinear();
}

export function shapeOf(points: [number, number][]): THREE.Shape {
  const shape = new THREE.Shape();
  points.forEach(([x, y], i) => (i ? shape.lineTo(x, y) : shape.moveTo(x, y)));
  shape.closePath();
  return shape;
}

export const rectPoints = (r: Rect): [number, number][] => [
  [r.x0, r.y0],
  [r.x1, r.y0],
  [r.x1, r.y1],
  [r.x0, r.y1],
];

/** Освобождает GPU-ресурсы поддерева (геометрии, материалы, их текстуры). */
export function disposeTree(root: THREE.Object3D) {
  root.traverse((o) => {
    const mesh = o as THREE.Mesh;
    mesh.geometry?.dispose();
    const materials = Array.isArray(mesh.material) ? mesh.material : mesh.material ? [mesh.material] : [];
    for (const m of materials) {
      (m as THREE.MeshBasicMaterial).map?.dispose();
      m.dispose();
    }
  });
}

/** Помечает меш токеном цвета темы: paintTokens перекрашивает его при смене темы. */
export function themed<T extends THREE.Mesh | THREE.LineSegments>(mesh: T, token: string): T {
  mesh.userData.token = token;
  return mesh;
}

/** Перекрашивает помеченные themed меши поддерева в текущие цвета темы. */
export function paintTokens(root: THREE.Object3D) {
  root.traverse((o) => {
    const token = o.userData.token as string | undefined;
    if (token) ((o as THREE.Mesh).material as THREE.MeshBasicMaterial).color.copy(cssColor(token));
  });
}

const UP = new THREE.Vector3(0, 0, 1);
const UNIT = new THREE.Vector3(1, 1, 1);

/** Матрица копии на столе: сдвиг и поворот вокруг вертикали (градусы против часовой). */
export function placementMatrix(target: THREE.Matrix4, x: number, y: number, z: number, angleRad: number) {
  return target.compose(new THREE.Vector3(x, y, z), new THREE.Quaternion().setFromAxisAngle(UP, angleRad), UNIT);
}

export const degToRad = (deg: number) => (deg * Math.PI) / 180;

/** Высота «спрятанного» экземпляра: за дальней плоскостью камеры. */
export const HIDDEN_Z = -9999;
