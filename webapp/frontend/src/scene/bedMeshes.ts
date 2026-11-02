import * as THREE from 'three';
import type { BedSpec, Rect } from './bed';
import { rectPoints, shapeOf, themed } from './three';

/** Толщина пластины и её опускание под рабочую поверхность, мм. */
const PLATE_DEPTH_MM = 3;
const PLATE_Z = -3.8;
/** Слои стола в десятых долях мм друг над другом: запретная зона, рабочая область, сетка, исключения. */
const FORBIDDEN_Z = -0.3;
const USABLE_Z = -0.2;
const GRID_Z = -0.1;
const EXCLUSION_Z = 0.02;
export const TOWER_Z = 0.04;
/** Шаг сетки рабочей области, мм. */
const GRID_STEP_MM = 32;
/** Засечки размерных линий, мм от линии. */
const DIM_TICK_MM = 4;
const FORBIDDEN_OPACITY = 0.55;
const EXCLUSION_OPACITY = 0.75;
const GRID_OPACITY = 0.7;
export const TOWER_OPACITY = 0.35;

const lines = (points: number[], material: THREE.LineBasicMaterial) =>
  new THREE.LineSegments(
    new THREE.BufferGeometry().setAttribute('position', new THREE.Float32BufferAttribute(points, 3)),
    material
  );

const flatRect = (rect: Rect, opacity: number) =>
  new THREE.Mesh(
    new THREE.ShapeGeometry(shapeOf(rectPoints(rect))),
    new THREE.MeshBasicMaterial({ transparent: true, opacity, depthWrite: false })
  );

/** Зона башни очистки поверх стола. */
export function towerMesh(tower: Rect): THREE.Mesh {
  const mesh = flatRect(tower, TOWER_OPACITY);
  mesh.position.z = TOWER_Z;
  return mesh;
}

/** Пластина, запретная зона, рабочая область, сетка, исключения и размерные линии; цвета — токены темы. */
export function bedGroup(bed: BedSpec): THREE.Group {
  const group = new THREE.Group();
  const plate = themed(
    new THREE.Mesh(
      new THREE.ExtrudeGeometry(shapeOf(bed.plate), { depth: PLATE_DEPTH_MM, bevelEnabled: false }),
      new THREE.MeshLambertMaterial()
    ),
    '--scene-plate'
  );
  plate.position.z = PLATE_Z;
  // Без вырезов (край области может совпадать с краем пластины — триангуляция ломается):
  // вся пластина цвета запретной зоны, рабочая область поверх.
  const forbidden = themed(
    new THREE.Mesh(
      new THREE.ShapeGeometry(shapeOf(bed.plate)),
      new THREE.MeshBasicMaterial({ transparent: true, opacity: FORBIDDEN_OPACITY, depthWrite: false })
    ),
    '--scene-keepout'
  );
  forbidden.position.z = FORBIDDEN_Z;
  const top = themed(
    new THREE.Mesh(new THREE.ShapeGeometry(shapeOf(rectPoints(bed.usable))), new THREE.MeshBasicMaterial()),
    '--scene-plate-top'
  );
  top.position.z = USABLE_Z;
  group.add(plate, forbidden, top);

  const u = bed.usable;
  const grid: number[] = [];
  for (let x = Math.ceil(u.x0 / GRID_STEP_MM) * GRID_STEP_MM; x <= u.x1; x += GRID_STEP_MM)
    grid.push(x, u.y0, 0, x, u.y1, 0);
  for (let y = Math.ceil(u.y0 / GRID_STEP_MM) * GRID_STEP_MM; y <= u.y1; y += GRID_STEP_MM)
    grid.push(u.x0, y, 0, u.x1, y, 0);
  const gridLines = themed(
    lines(grid, new THREE.LineBasicMaterial({ transparent: true, opacity: GRID_OPACITY })),
    '--scene-plate-grid'
  );
  gridLines.position.z = GRID_Z;
  group.add(gridLines);

  for (const rect of bed.exclusions) {
    const exclusion = themed(flatRect(rect, EXCLUSION_OPACITY), '--scene-keepout');
    exclusion.position.z = EXCLUSION_Z;
    group.add(exclusion);
  }

  // Размерные линии вдоль передней и правой сторон, с засечками на концах.
  const [front, side] = bed.labels;
  const a = bed.area;
  const t = DIM_TICK_MM;
  // prettier-ignore
  const dims = [
    a.x0, front.y, 0, a.x1, front.y, 0,
    a.x0, front.y - t, 0, a.x0, front.y + t, 0,
    a.x1, front.y - t, 0, a.x1, front.y + t, 0,
    side.x, a.y0, 0, side.x, a.y1, 0,
    side.x - t, a.y0, 0, side.x + t, a.y0, 0,
    side.x - t, a.y1, 0, side.x + t, a.y1, 0,
  ];
  group.add(themed(lines(dims, new THREE.LineBasicMaterial()), '--dim'));
  return group;
}
