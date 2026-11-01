import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import Icon from '@/components/Icon';
import * as THREE from 'three';
import { OrbitControls } from 'three/addons/controls/OrbitControls.js';
import type { PrinterProfile } from '@/api/types';
import { useUiStore } from '@/store/useUiStore';
import { bedSpec, type BedSpec, type Rect } from './bed';
import type { Placement } from './layout';
import type { ModelMesh } from './model3d';
import { projectTriangles, type ProjectionMask } from './projection';

export interface BedSceneProps {
  profile: PrinterProfile;
  model: ModelMesh | null;
  /** idle — предпросмотр; running — задача идёт; done — готово (копии без падения). */
  mode: 'idle' | 'running' | 'done';
  /** Высота среза 0..1 (sceneClip); 1 — без среза. */
  clip: number;
  /** Позиции копий на столе (центр габарита модели, мм, угол против часовой). Пусто — одна модель в центре. */
  placements?: Placement[];
  /** Контур для подложки, мм, вокруг центра модели (из пакера); нет — проекция меша. */
  footprintLoops?: [number, number][][] | null;
  /** Зона башни очистки (из раскладки пакера) — принадлежит задаче, не принтеру. */
  tower?: Rect | null;
  /** Рабочая область из раскладки пакера (учтён отступ от края, заданный пользователем) — вместо
   *  области профиля; подписи стола остаются по профилю. */
  usableOverride?: Rect | null;
  /** Идёт перебор раскладок, позиций ещё нет: по столу мелькают проекции модели. */
  searching?: boolean;
  /** Подписи в углах сцены. */
  caption?: string;
  readout?: string;
  /** Сколько объектов получилось на столе — числом под легендой; null — итога ещё нет. */
  objectCount?: number | null;
}

const LAYER_MM = 0.2;
/** Падение копий на стол после раскладки: общая длительность и высота старта, мм. */
const DROP_MS = 1400;
const DROP_FROM_MM = 120;
/** Перебор раскладок: сколько проекций мелькает, шаг смены кадра и разброс вокруг места, мм. */
const GHOSTS = 48;
const GHOST_TICK_MS = 200;
const GHOST_JITTER_MM = 6;

/** oklch/hex CSS-переменная → THREE.Color (через canvas: THREE не понимает oklch). */
function cssColor(name: string): THREE.Color {
  const c = document.createElement('canvas');
  c.width = c.height = 1;
  const x = c.getContext('2d', { willReadFrequently: true })!;
  x.fillStyle = cssValue(name) || '#888';
  x.fillRect(0, 0, 1, 1);
  const d = x.getImageData(0, 0, 1, 1).data;
  return new THREE.Color(d[0] / 255, d[1] / 255, d[2] / 255).convertSRGBToLinear();
}
const cssValue = (name: string) => getComputedStyle(document.documentElement).getPropertyValue(name).trim();

function shapeOf(points: [number, number][]): THREE.Shape {
  const s = new THREE.Shape();
  points.forEach(([x, y], i) => (i ? s.lineTo(x, y) : s.moveTo(x, y)));
  s.closePath();
  return s;
}
const rectPoints = (r: { x0: number; y0: number; x1: number; y1: number }): [number, number][] => [
  [r.x0, r.y0],
  [r.x1, r.y0],
  [r.x1, r.y1],
  [r.x0, r.y1],
];

/** Освобождает GPU-ресурсы поддерева (геометрии, материалы, их текстуры). */
function disposeTree(root: THREE.Object3D) {
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

/** Источник подложки: контуры пакера (even-odd, их немного) или растр проекции меша. */
type FootprintSource = { kind: 'loops'; polygons: [number, number][][] } | { kind: 'mask'; projection: ProjectionMask };

/** Подложка «проекция + контур»: заливка 28 % и обводка 1.6 мм цветом --accent-2. */
function footprintTexture(source: FootprintSource): {
  texture: THREE.CanvasTexture;
  size: [number, number];
  center: [number, number];
} {
  const silhouette = document.createElement('canvas');
  let w: number, h: number, res: number, minX: number, minY: number, maxX: number, maxY: number;
  if (source.kind === 'mask') {
    const p = source.projection;
    ({ width: w, height: h, res, minX, minY, maxX, maxY } = p);
    silhouette.width = w;
    silhouette.height = h;
    const image = new ImageData(w, h);
    for (let i = 0; i < p.mask.length; i++) if (p.mask[i]) image.data.set([255, 255, 255, 255], i * 4);
    silhouette.getContext('2d')!.putImageData(image, 0, 0);
  } else {
    minX = minY = Infinity;
    maxX = maxY = -Infinity;
    for (const poly of source.polygons)
      for (const [x, y] of poly) {
        minX = Math.min(minX, x);
        maxX = Math.max(maxX, x);
        minY = Math.min(minY, y);
        maxY = Math.max(maxY, y);
      }
    if (!Number.isFinite(minX)) minX = minY = maxX = maxY = 0;
    res = 4;
    const pad = 3;
    w = Math.max(2, Math.ceil((maxX - minX + pad * 2) * res));
    h = Math.max(2, Math.ceil((maxY - minY + pad * 2) * res));
    const X = (x: number) => (x - minX + pad) * res,
      Y = (y: number) => h - (y - minY + pad) * res;
    silhouette.width = w;
    silhouette.height = h;
    const sx = silhouette.getContext('2d')!;
    sx.fillStyle = '#fff';
    sx.beginPath();
    for (const poly of source.polygons)
      poly.forEach(([x, y], i) => (i ? sx.lineTo(X(x), Y(y)) : sx.moveTo(X(x), Y(y))));
    sx.fill('evenodd');
  }
  const tint = (color: string) => {
    const c = document.createElement('canvas');
    c.width = w;
    c.height = h;
    const x = c.getContext('2d')!;
    x.drawImage(silhouette, 0, 0);
    x.globalCompositeOperation = 'source-in';
    x.fillStyle = color;
    x.fillRect(0, 0, w, h);
    return c;
  };
  const out = document.createElement('canvas');
  out.width = w;
  out.height = h;
  const ox = out.getContext('2d')!;
  const ring = tint(cssValue('--accent-2')),
    r = 0.8 * res;
  for (let a = 0; a < 16; a++) ox.drawImage(ring, Math.cos((a / 8) * Math.PI) * r, Math.sin((a / 8) * Math.PI) * r);
  ox.globalCompositeOperation = 'destination-out';
  ox.drawImage(silhouette, 0, 0);
  ox.globalCompositeOperation = 'source-over';
  ox.globalAlpha = 0.28;
  ox.drawImage(ring, 0, 0);
  ox.globalAlpha = 1;
  const texture = new THREE.CanvasTexture(out);
  texture.colorSpace = THREE.SRGBColorSpace;
  return { texture, size: [w / res, h / res], center: [(minX + maxX) / 2, (minY + maxY) / 2] };
}

interface SceneApi {
  rebuild: (theme: string) => void;
  invalidate: () => void;
  resetView: () => void;
}

export default function BedScene(props: BedSceneProps) {
  const hostRef = useRef<HTMLDivElement>(null);
  const labelsRef = useRef<HTMLDivElement>(null);
  const propsRef = useRef(props);
  const theme = useUiStore((s) => s.theme);
  const apiRef = useRef<SceneApi | null>(null);
  /** Вид менялся мышью — показать «Сбросить вид». */
  const [moved, setMoved] = useState(false);

  // Свежие пропсы для кадра; кадр рисуется по запросу (mode/clip/подписи меняются только через пропсы).
  useLayoutEffect(() => {
    propsRef.current = props;
    apiRef.current?.invalidate();
  });

  useEffect(() => {
    const host = hostRef.current!;
    const renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true });
    renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
    renderer.localClippingEnabled = true;
    host.prepend(renderer.domElement);

    const scene = new THREE.Scene();
    // near/far 1..2000: шире — z-fighting слоёв стола (они в десятых долях мм друг от друга).
    const camera = new THREE.OrthographicCamera(-1, 1, 1, -1, 1, 2000);
    camera.up.set(0, 0, 1);
    scene.add(new THREE.HemisphereLight(0xffffff, 0x666666, 1.6));
    const sun = new THREE.DirectionalLight(0xffffff, 1.5);
    sun.position.set(-150, -260, 420);
    scene.add(sun);
    // Вращение, масштаб колесом (к курсору — рассмотреть угол стола) и сдвиг правой кнопкой — всегда,
    // а не только на готовом столе: идущую нарезку тоже хочется разглядеть.
    const controls = new OrbitControls(camera, renderer.domElement);
    controls.zoomToCursor = true;
    controls.minZoom = 0.7;
    controls.maxZoom = 12;
    /** Камера в исходной точке обзора; false — поставить заново (новый стол, «Сбросить вид»). */
    let viewSet = false;

    const clipPlane = new THREE.Plane(new THREE.Vector3(0, 0, -1), 1e4);
    const objectMaterial = new THREE.MeshLambertMaterial({ clippingPlanes: [clipPlane] });
    // Внутренние грани, видимые сквозь срез, — сигнальным цветом: «горячий» срез слоя.
    const capMaterial = new THREE.MeshBasicMaterial({ side: THREE.BackSide, clippingPlanes: [clipPlane] });
    const sliceMaterial = new THREE.MeshBasicMaterial({
      transparent: true,
      opacity: 0.06,
      depthWrite: false,
      side: THREE.DoubleSide,
    });
    const slicePlane = new THREE.Mesh(new THREE.PlaneGeometry(1, 1), sliceMaterial);
    scene.add(slicePlane);

    let bed: BedSpec | null = null;
    let bedGroup = new THREE.Group();
    let modelGeometry: THREE.BufferGeometry | null = null;
    let solid: THREE.InstancedMesh | null = null;
    let cap: THREE.InstancedMesh | null = null;
    let footprints: THREE.InstancedMesh | null = null;
    /** Проекции модели для перебора раскладок; места — сетка по рабочей области. */
    let ghosts: THREE.InstancedMesh | null = null;
    let ghostSpots: { x: number; y: number }[] = [];
    let ghostTimer = 0;
    let towerMesh: THREE.Mesh | null = null;
    let footprintSource: FootprintSource | null = null;
    let height = 1;
    let spots: Placement[] = [];
    let dropStart = 0;
    let built = {
      profile: null as PrinterProfile | null,
      usable: '',
      model: null as ModelMesh | null,
      placements: '',
      copies: '',
      loops: null as unknown,
      theme: '',
    };

    const colored = <T extends THREE.Mesh | THREE.LineSegments>(mesh: T, token: string): T => {
      mesh.userData.token = token;
      return mesh;
    };
    const paint = () => {
      bedGroup.traverse((o) => {
        const token = o.userData.token as string | undefined;
        if (token) ((o as THREE.Mesh).material as THREE.MeshBasicMaterial).color.copy(cssColor(token));
      });
      if (towerMesh) (towerMesh.material as THREE.MeshBasicMaterial).color.copy(cssColor('--accent-2'));
      objectMaterial.color.copy(cssColor('--object'));
      capMaterial.color.copy(cssColor('--signal'));
      sliceMaterial.color.copy(cssColor('--signal'));
    };

    const buildBed = (profile: PrinterProfile, usableOverride: Rect | null) => {
      scene.remove(bedGroup);
      disposeTree(bedGroup);
      bedGroup = new THREE.Group();
      const spec = bedSpec(profile);
      // Отступ от края пользователь может задать свой: реальная рабочая область — из раскладки.
      bed = usableOverride ? { ...spec, usable: usableOverride } : spec;
      const plate = colored(
        new THREE.Mesh(
          new THREE.ExtrudeGeometry(shapeOf(bed.plate), { depth: 3, bevelEnabled: false }),
          new THREE.MeshLambertMaterial()
        ),
        '--plate'
      );
      plate.position.z = -3.8;
      // Без вырезов (край области может совпадать с краем пластины — триангуляция ломается):
      // вся пластина цвета запретной зоны, рабочая область поверх.
      const forbidden = colored(
        new THREE.Mesh(
          new THREE.ShapeGeometry(shapeOf(bed.plate)),
          new THREE.MeshBasicMaterial({ transparent: true, opacity: 0.55, depthWrite: false })
        ),
        '--keepout'
      );
      forbidden.position.z = -0.3;
      const top = colored(
        new THREE.Mesh(new THREE.ShapeGeometry(shapeOf(rectPoints(bed.usable))), new THREE.MeshBasicMaterial()),
        '--plate-top'
      );
      top.position.z = -0.2;
      bedGroup.add(plate, forbidden, top);
      const u = bed.usable,
        grid: number[] = [];
      for (let x = Math.ceil(u.x0 / 32) * 32; x <= u.x1; x += 32) grid.push(x, u.y0, 0, x, u.y1, 0);
      for (let y = Math.ceil(u.y0 / 32) * 32; y <= u.y1; y += 32) grid.push(u.x0, y, 0, u.x1, y, 0);
      const gridLines = colored(
        new THREE.LineSegments(
          new THREE.BufferGeometry().setAttribute('position', new THREE.Float32BufferAttribute(grid, 3)),
          new THREE.LineBasicMaterial({ transparent: true, opacity: 0.7 })
        ),
        '--plate-grid'
      );
      gridLines.position.z = -0.1;
      bedGroup.add(gridLines);
      for (const r of bed.exclusions) {
        const ex = colored(
          new THREE.Mesh(
            new THREE.ShapeGeometry(shapeOf(rectPoints(r))),
            new THREE.MeshBasicMaterial({ transparent: true, opacity: 0.75, depthWrite: false })
          ),
          '--keepout'
        );
        ex.position.z = 0.02;
        bedGroup.add(ex);
      }
      // Размерные линии вдоль передней и правой сторон.
      const [front, side] = bed.labels;
      const a = bed.area,
        dims = [
          a.x0,
          front.y,
          0,
          a.x1,
          front.y,
          0,
          a.x0,
          front.y - 4,
          0,
          a.x0,
          front.y + 4,
          0,
          a.x1,
          front.y - 4,
          0,
          a.x1,
          front.y + 4,
          0,
          side.x,
          a.y0,
          0,
          side.x,
          a.y1,
          0,
          side.x - 4,
          a.y0,
          0,
          side.x + 4,
          a.y0,
          0,
          side.x - 4,
          a.y1,
          0,
          side.x + 4,
          a.y1,
          0,
        ];
      bedGroup.add(
        colored(
          new THREE.LineSegments(
            new THREE.BufferGeometry().setAttribute('position', new THREE.Float32BufferAttribute(dims, 3)),
            new THREE.LineBasicMaterial()
          ),
          '--fg-dim'
        )
      );
      scene.add(bedGroup);
    };

    /** Текстура подложки зависит от --accent-2: перерисовывается при смене темы без перестройки меша. */
    const paintFootprints = () => {
      if (!footprintSource) return;
      for (const mesh of [footprints, ghosts]) {
        if (!mesh) continue;
        const material = mesh.material as THREE.MeshBasicMaterial;
        material.map?.dispose();
        material.map = footprintTexture(footprintSource).texture;
        material.needsUpdate = true;
      }
    };

    const disposeObjects = () => {
      for (const o of [solid, cap, footprints, ghosts])
        if (o) {
          scene.remove(o);
          o.dispose();
        }
      if (towerMesh) {
        scene.remove(towerMesh);
        disposeTree(towerMesh);
      }
      for (const mesh of [footprints, ghosts])
        if (mesh) {
          mesh.geometry.dispose();
          const material = mesh.material as THREE.MeshBasicMaterial;
          material.map?.dispose();
          material.dispose();
        }
      modelGeometry?.dispose(); // общая для solid и cap; материалы общие на всю сцену
      modelGeometry = null;
      solid = cap = footprints = ghosts = null;
      ghostSpots = [];
      towerMesh = null;
      footprintSource = null;
      dropStart = 0;
    };

    /** Матрицы копий: progress 0..1 — время падения (1 — все на столе). */
    const placeCopies = (progress: number) => {
      if (!solid || !cap) return;
      const m = new THREE.Matrix4(),
        q = new THREE.Quaternion(),
        s = new THREE.Vector3(1, 1, 1),
        z = new THREE.Vector3(0, 0, 1);
      spots.forEach((p, i) => {
        // По порядку пакера — от центра наружу; каждая копия падает за ~40 % общего времени.
        const local = progress >= 1 ? 1 : Math.min(1, Math.max(0, (progress - (i / spots.length) * 0.6) / 0.4));
        const eased = 1 - (1 - local) ** 3;
        q.setFromAxisAngle(z, (p.angleDeg * Math.PI) / 180);
        // Ещё не начавшая падать копия — за дальней плоскостью камеры.
        m.compose(new THREE.Vector3(p.x, p.y, progress >= 1 || local > 0 ? (1 - eased) * DROP_FROM_MM : -9999), q, s);
        solid!.setMatrixAt(i, m);
        cap!.setMatrixAt(i, m);
      });
      solid.instanceMatrix.needsUpdate = true;
      cap.instanceMatrix.needsUpdate = true;
    };

    const buildObjects = (drop: boolean) => {
      disposeObjects();
      const { model, placements = [], footprintLoops, tower } = propsRef.current;
      if (tower) {
        towerMesh = new THREE.Mesh(
          new THREE.ShapeGeometry(shapeOf(rectPoints(tower))),
          new THREE.MeshBasicMaterial({ transparent: true, opacity: 0.35, depthWrite: false })
        );
        towerMesh.position.z = 0.04;
        scene.add(towerMesh);
      }
      if (!model || !bed) return;
      const geometry = new THREE.BufferGeometry().setAttribute(
        'position',
        new THREE.BufferAttribute(model.positions, 3)
      );
      geometry.computeVertexNormals();
      modelGeometry = geometry;
      height = Math.max(model.size[2], 0.2);
      spots = placements.length
        ? placements
        : [{ x: (bed.usable.x0 + bed.usable.x1) / 2, y: (bed.usable.y0 + bed.usable.y1) / 2, angleDeg: 0 }];
      solid = new THREE.InstancedMesh(geometry, objectMaterial, spots.length);
      cap = new THREE.InstancedMesh(geometry, capMaterial, spots.length);
      // Сфера отсечения InstancedMesh считается один раз — по кадру падения она была бы неверной.
      solid.frustumCulled = cap.frustumCulled = false;
      footprintSource = footprintLoops?.length
        ? { kind: 'loops', polygons: footprintLoops }
        : { kind: 'mask', projection: model.projection ?? projectTriangles(model.positions, model.triangles) };
      const fp = footprintTexture(footprintSource);
      footprints = new THREE.InstancedMesh(
        new THREE.PlaneGeometry(fp.size[0], fp.size[1]),
        new THREE.MeshBasicMaterial({ map: fp.texture, transparent: true, depthWrite: false }),
        spots.length
      );
      const m = new THREE.Matrix4(),
        q = new THREE.Quaternion(),
        s = new THREE.Vector3(1, 1, 1),
        z = new THREE.Vector3(0, 0, 1);
      spots.forEach((p, i) => {
        // Контур — сразу на месте: копии падают на свои подложки.
        q.setFromAxisAngle(z, (p.angleDeg * Math.PI) / 180);
        const offset = new THREE.Vector3(fp.center[0], fp.center[1], 0).applyQuaternion(q);
        m.compose(new THREE.Vector3(p.x + offset.x, p.y + offset.y, 0.06), q, s);
        footprints!.setMatrixAt(i, m);
      });
      placeCopies(drop ? 0 : 1);
      dropStart = drop ? performance.now() : 0;
      scene.add(solid, cap, footprints);
      // Перебор раскладок (v2-макет): проекции модели по сетке рабочей области, кадр раз в 200 мс.
      const cell = Math.max(fp.size[0], fp.size[1]) * 0.9;
      const u = bed.usable;
      const columns = Math.max(1, Math.floor((u.x1 - u.x0) / cell));
      const rows = Math.max(1, Math.floor((u.y1 - u.y0) / cell));
      for (let r = 0; r < rows; r++)
        for (let c = 0; c < columns; c++)
          ghostSpots.push({
            x: u.x0 + ((c + 0.5 + (r % 2) * 0.25) * (u.x1 - u.x0)) / columns,
            y: u.y0 + ((r + 0.5) * (u.y1 - u.y0)) / rows,
          });
      ghosts = new THREE.InstancedMesh(
        new THREE.PlaneGeometry(fp.size[0], fp.size[1]),
        new THREE.MeshBasicMaterial({
          map: footprintTexture(footprintSource).texture,
          transparent: true,
          opacity: 0.7,
          depthWrite: false,
        }),
        Math.min(GHOSTS, ghostSpots.length)
      );
      ghosts.frustumCulled = false;
      ghosts.visible = false;
      scene.add(ghosts);
    };

    /** Кадр перебора: каждая четвёртая проекция на своём месте со сдвигом и поворотом, остальные скрыты. */
    const placeGhosts = (tick: number) => {
      if (!ghosts) return;
      const m = new THREE.Matrix4(),
        q = new THREE.Quaternion(),
        s = new THREE.Vector3(1, 1, 1),
        z = new THREE.Vector3(0, 0, 1);
      for (let i = 0; i < ghosts.count; i++) {
        const spot = ghostSpots[(i * 7) % ghostSpots.length];
        const on = (i * 7 + tick) % 4 === 0;
        q.setFromAxisAngle(z, ((tick + i) % 4) * 0.5);
        m.compose(
          new THREE.Vector3(
            spot.x + Math.sin(tick + i) * GHOST_JITTER_MM,
            spot.y + Math.cos(tick * 2 + i) * GHOST_JITTER_MM,
            on ? 0.07 : -9999
          ),
          q,
          s
        );
        ghosts.setMatrixAt(i, m);
      }
      ghosts.instanceMatrix.needsUpdate = true;
    };

    const fit = () => {
      const w = host.clientWidth,
        h = host.clientHeight;
      if (!w || !h || !bed) return;
      renderer.setSize(w, h, false);
      const cx = (bed.area.x0 + bed.area.x1) / 2,
        cy = (bed.area.y0 + bed.area.y1) / 2;
      const half = Math.max(bed.area.x1 - bed.area.x0, bed.area.y1 - bed.area.y0) * 0.5,
        aspect = w / h;
      camera.left = -half * aspect;
      camera.right = half * aspect;
      camera.top = half;
      camera.bottom = -half;
      if (!viewSet) {
        camera.position.set(cx + 420, cy - 420, 360);
        controls.target.set(cx, cy, 0);
        camera.zoom = 1;
        camera.lookAt(cx, cy, 0);
        viewSet = true;
      }
      camera.updateProjectionMatrix();
      controls.update();
    };

    const v = new THREE.Vector3();
    let frame = 0;
    const draw = () => {
      frame = 0;
      const p = propsRef.current;
      let dropping = false;
      if (dropStart) {
        const t = (performance.now() - dropStart) / DROP_MS;
        placeCopies(t);
        if (t >= 1) dropStart = 0;
        else dropping = true;
      }
      // Перебор раскладок: вместо одиночной модели — мелькающие проекции; следующий кадр — по таймеру,
      // а не постоянным циклом. prefers-reduced-motion — один неподвижный кадр.
      const searching = Boolean(p.searching && ghosts && !p.placements?.length);
      if (ghosts) ghosts.visible = searching;
      for (const o of [solid, cap, footprints]) if (o) o.visible = !searching;
      window.clearTimeout(ghostTimer);
      if (searching) {
        placeGhosts(reducedMotion.matches ? 0 : Math.floor(performance.now() / GHOST_TICK_MS));
        if (!reducedMotion.matches) ghostTimer = window.setTimeout(invalidate, GHOST_TICK_MS);
      }
      // Падение — показ готовой раскладки: срез слоя на это время снят, иначе копии в воздухе не видны.
      // clip = 1 — без среза: плоскость ровно на верхней грани даёт z-fighting верха копий.
      const clipZ = p.mode === 'running' && !dropping && p.clip < 1 ? Math.max(LAYER_MM, p.clip * height) : 1e4;
      clipPlane.constant = clipZ;
      // Без среза изнанка не нужна: иначе её край просвечивает оранжевым по контуру модели.
      if (cap) cap.visible = !searching && clipZ < height;
      slicePlane.visible = p.mode === 'running' && !dropping && p.clip < 1 && bed !== null;
      if (bed && slicePlane.visible) {
        const a = bed.area;
        slicePlane.scale.set(a.x1 - a.x0 + 16, a.y1 - a.y0 + 16, 1);
        slicePlane.position.set((a.x0 + a.x1) / 2, (a.y0 + a.y1) / 2, clipZ);
      }
      renderer.render(scene, camera);
      if (labelsRef.current && bed) {
        const w = host.clientWidth,
          h = host.clientHeight;
        labelsRef.current.replaceChildren(
          ...bed.labels.map((l) => {
            v.set(l.x, l.y, 0).project(camera);
            const el = document.createElement('span');
            el.className = 'scene-dim';
            el.style.left = `${((v.x + 1) / 2) * w}px`;
            el.style.top = `${((1 - v.y) / 2) * h}px`;
            el.textContent = l.text;
            return el;
          })
        );
      }
      if (dropping) invalidate();
    };
    // Постоянного цикла нет: кадр — по смене пропсов, размера, вращению мышью и, пока идёт, падению копий.
    const invalidate = () => {
      if (!frame) frame = requestAnimationFrame(draw);
    };
    const reducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)');

    const rebuild = (themeKey: string) => {
      const p = propsRef.current;
      const usable = JSON.stringify(p.usableOverride ?? null);
      const profileChanged = p.profile !== built.profile || usable !== built.usable;
      if (profileChanged) {
        buildBed(p.profile, p.usableOverride ?? null);
        if (p.profile !== built.profile) viewSet = false;
      }
      const key = JSON.stringify([p.placements ?? [], p.tower ?? null]);
      const copies = JSON.stringify(p.placements ?? []);
      const objectsChanged =
        profileChanged || p.model !== built.model || key !== built.placements || p.footprintLoops !== built.loops;
      // Падают только новые копии задачи, которая ещё идёт: после успеха (в т. ч. перезагрузка
      // страницы) и при prefers-reduced-motion сразу итоговый стол.
      const drop =
        copies !== built.copies && Boolean(p.placements?.length) && p.mode !== 'done' && !reducedMotion.matches;
      if (objectsChanged) buildObjects(drop);
      else if (themeKey !== built.theme) paintFootprints();
      built = {
        profile: p.profile,
        usable,
        model: p.model,
        placements: key,
        copies,
        loops: p.footprintLoops,
        theme: themeKey,
      };
      paint();
      fit();
      invalidate();
    };
    const resetView = () => {
      viewSet = false;
      fit();
      invalidate();
    };
    apiRef.current = { rebuild, invalidate, resetView };
    const moved = () => setMoved(true);
    controls.addEventListener('start', moved);
    const observer = new ResizeObserver(() => {
      fit();
      invalidate();
    });
    observer.observe(host);
    controls.addEventListener('change', invalidate);

    return () => {
      apiRef.current = null;
      cancelAnimationFrame(frame);
      window.clearTimeout(ghostTimer);
      observer.disconnect();
      controls.removeEventListener('change', invalidate);
      controls.removeEventListener('start', moved);
      controls.dispose();
      disposeObjects();
      disposeTree(bedGroup);
      slicePlane.geometry.dispose();
      sliceMaterial.dispose();
      objectMaterial.dispose();
      capMaterial.dispose();
      // Контекст отпускается сразу, не дожидаясь GC: иначе после нескольких заходов на страницу
      // браузер упирается в лимит WebGL-контекстов и роняет самый старый.
      renderer.forceContextLoss();
      renderer.dispose();
      renderer.domElement.remove();
    };
  }, []);

  // Перестройка при смене профиля/модели/раскладки и перекраска при смене темы
  // (тема уже применена к <html> до обновления стора — цвета токенов читаются свежими).
  useEffect(() => {
    apiRef.current?.rebuild(theme);
  }, [props.profile, props.usableOverride, props.model, props.placements, props.footprintLoops, props.tower, theme]);

  return (
    <div className="scene" ref={hostRef}>
      <div className="scene-labels" ref={labelsRef} />
      <div className="scene-legend">
        <span>
          непечатная зона <i className="swatch swatch-keepout" />
        </span>
        <span>
          контур проекции <i className="swatch swatch-footprint" />
        </span>
        {props.tower && (
          <span>
            башня очистки <i className="swatch swatch-tower" />
          </span>
        )}
        {props.objectCount != null && (
          <b className="scene-count" aria-label={`Объектов на столе: ${props.objectCount}`}>
            {props.objectCount}
          </b>
        )}
      </div>
      {props.caption && <div className="scene-tag scene-tag-bl">{props.caption}</div>}
      {props.readout && <div className="scene-tag scene-tag-br">{props.readout}</div>}
      {moved ? (
        <button
          type="button"
          className="icon-btn bordered scene-reset"
          aria-label="Сбросить вид"
          data-tip="Сбросить вид"
          onClick={() => {
            apiRef.current?.resetView();
            setMoved(false);
          }}
        >
          <Icon name="focus" />
        </button>
      ) : null}
    </div>
  );
}
