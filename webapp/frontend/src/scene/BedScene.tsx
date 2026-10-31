import { useEffect, useLayoutEffect, useRef } from 'react';
import * as THREE from 'three';
import { OrbitControls } from 'three/addons/controls/OrbitControls.js';
import type { PrinterProfile } from '@/api/types';
import { useUiStore } from '@/store/useUiStore';
import { bedSpec, type BedSpec } from './bed';
import type { ModelMesh } from './model3d';

export interface Placement { x: number; y: number; angleDeg: number }

export interface BedSceneProps {
  profile: PrinterProfile;
  model: ModelMesh | null;
  /** idle — предпросмотр; running — задача идёт; done — готово (вращение мышью). */
  mode: 'idle' | 'running' | 'done';
  /** Высота среза 0..1 (sceneClip); 1 — без среза. */
  clip: number;
  /** Позиции копий на столе (центр габарита модели, мм, угол против часовой). Пусто — одна модель в центре. */
  placements?: Placement[];
  /** Контур для подложки, мм, вокруг центра модели (из пакера); нет — проекция меша. */
  footprintLoops?: [number, number][][] | null;
  /** Подписи в углах сцены. */
  caption?: string;
  readout?: string;
}

const LAYER_MM = 0.2;

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
const rectPoints = (r: { x0: number; y0: number; x1: number; y1: number }): [number, number][] =>
  [[r.x0, r.y0], [r.x1, r.y0], [r.x1, r.y1], [r.x0, r.y1]];

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

/** Подложка «проекция + контур»: заливка 28 % и обводка 1.6 мм цветом --accent-2. */
function footprintTexture(polygons: [number, number][][], evenOdd: boolean): { texture: THREE.CanvasTexture; size: [number, number]; center: [number, number] } {
  let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
  for (const poly of polygons) for (const [x, y] of poly) { minX = Math.min(minX, x); maxX = Math.max(maxX, x); minY = Math.min(minY, y); maxY = Math.max(maxY, y); }
  if (!Number.isFinite(minX)) minX = minY = maxX = maxY = 0;
  const res = 4, pad = 3;
  const w = Math.max(2, Math.ceil((maxX - minX + pad * 2) * res)), h = Math.max(2, Math.ceil((maxY - minY + pad * 2) * res));
  const X = (x: number) => (x - minX + pad) * res, Y = (y: number) => h - (y - minY + pad) * res;
  const silhouette = document.createElement('canvas');
  silhouette.width = w; silhouette.height = h;
  const sx = silhouette.getContext('2d')!;
  sx.fillStyle = sx.strokeStyle = '#fff';
  if (evenOdd) {
    sx.beginPath();
    for (const poly of polygons) poly.forEach(([x, y], i) => (i ? sx.lineTo(X(x), Y(y)) : sx.moveTo(X(x), Y(y))));
    sx.fill('evenodd');
  } else {
    // Треугольники меша по одному: обводка закрывает щели антиалиасинга между ними.
    for (const poly of polygons) { sx.beginPath(); poly.forEach(([x, y], i) => (i ? sx.lineTo(X(x), Y(y)) : sx.moveTo(X(x), Y(y)))); sx.closePath(); sx.fill(); sx.stroke(); }
  }
  const tint = (color: string) => {
    const c = document.createElement('canvas'); c.width = w; c.height = h;
    const x = c.getContext('2d')!; x.drawImage(silhouette, 0, 0);
    x.globalCompositeOperation = 'source-in'; x.fillStyle = color; x.fillRect(0, 0, w, h); return c;
  };
  const out = document.createElement('canvas'); out.width = w; out.height = h;
  const ox = out.getContext('2d')!;
  const ring = tint(cssValue('--accent-2')), r = 0.8 * res;
  for (let a = 0; a < 16; a++) ox.drawImage(ring, Math.cos((a / 8) * Math.PI) * r, Math.sin((a / 8) * Math.PI) * r);
  ox.globalCompositeOperation = 'destination-out'; ox.drawImage(silhouette, 0, 0);
  ox.globalCompositeOperation = 'source-over'; ox.globalAlpha = 0.28; ox.drawImage(ring, 0, 0); ox.globalAlpha = 1;
  const texture = new THREE.CanvasTexture(out);
  texture.colorSpace = THREE.SRGBColorSpace;
  return { texture, size: [w / res, h / res], center: [(minX + maxX) / 2, (minY + maxY) / 2] };
}

interface SceneApi {
  rebuild: (theme: string) => void;
  invalidate: () => void;
}

export default function BedScene(props: BedSceneProps) {
  const hostRef = useRef<HTMLDivElement>(null);
  const labelsRef = useRef<HTMLDivElement>(null);
  const propsRef = useRef(props);
  const theme = useUiStore((s) => s.theme);
  const apiRef = useRef<SceneApi | null>(null);

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
    const controls = new OrbitControls(camera, renderer.domElement);
    controls.enablePan = false;
    controls.enabled = false;

    const clipPlane = new THREE.Plane(new THREE.Vector3(0, 0, -1), 1e4);
    const objectMaterial = new THREE.MeshLambertMaterial({ clippingPlanes: [clipPlane] });
    // Внутренние грани, видимые сквозь срез, — сигнальным цветом: «горячий» срез слоя.
    const capMaterial = new THREE.MeshBasicMaterial({ side: THREE.BackSide, clippingPlanes: [clipPlane] });
    const sliceMaterial = new THREE.MeshBasicMaterial({ transparent: true, opacity: 0.06, depthWrite: false, side: THREE.DoubleSide });
    const slicePlane = new THREE.Mesh(new THREE.PlaneGeometry(1, 1), sliceMaterial);
    scene.add(slicePlane);

    let bed: BedSpec | null = null;
    let bedGroup = new THREE.Group();
    let modelGeometry: THREE.BufferGeometry | null = null;
    let solid: THREE.InstancedMesh | null = null;
    let cap: THREE.InstancedMesh | null = null;
    let footprints: THREE.InstancedMesh | null = null;
    let footprintSource: { polygons: [number, number][][]; evenOdd: boolean } | null = null;
    let height = 1;
    let built = { profile: null as PrinterProfile | null, model: null as ModelMesh | null, placements: '', loops: null as unknown, theme: '' };

    const colored = <T extends THREE.Mesh | THREE.LineSegments>(mesh: T, token: string): T => {
      mesh.userData.token = token;
      return mesh;
    };
    const paint = () => {
      bedGroup.traverse((o) => {
        const token = o.userData.token as string | undefined;
        if (token) ((o as THREE.Mesh).material as THREE.MeshBasicMaterial).color.copy(cssColor(token));
      });
      objectMaterial.color.copy(cssColor('--object'));
      capMaterial.color.copy(cssColor('--signal'));
      sliceMaterial.color.copy(cssColor('--signal'));
    };

    const buildBed = (profile: PrinterProfile) => {
      scene.remove(bedGroup);
      disposeTree(bedGroup);
      bedGroup = new THREE.Group();
      bed = bedSpec(profile);
      const plate = colored(new THREE.Mesh(new THREE.ExtrudeGeometry(shapeOf(bed.plate), { depth: 3, bevelEnabled: false }), new THREE.MeshLambertMaterial()), '--plate');
      plate.position.z = -3.8;
      // Без вырезов (край области может совпадать с краем пластины — триангуляция ломается):
      // вся пластина цвета запретной зоны, рабочая область поверх.
      const forbidden = colored(new THREE.Mesh(new THREE.ShapeGeometry(shapeOf(bed.plate)), new THREE.MeshBasicMaterial({ transparent: true, opacity: 0.55, depthWrite: false })), '--keepout');
      forbidden.position.z = -0.3;
      const top = colored(new THREE.Mesh(new THREE.ShapeGeometry(shapeOf(rectPoints(bed.usable))), new THREE.MeshBasicMaterial()), '--plate-top');
      top.position.z = -0.2;
      bedGroup.add(plate, forbidden, top);
      const u = bed.usable, grid: number[] = [];
      for (let x = Math.ceil(u.x0 / 32) * 32; x <= u.x1; x += 32) grid.push(x, u.y0, 0, x, u.y1, 0);
      for (let y = Math.ceil(u.y0 / 32) * 32; y <= u.y1; y += 32) grid.push(u.x0, y, 0, u.x1, y, 0);
      const gridLines = colored(new THREE.LineSegments(new THREE.BufferGeometry().setAttribute('position', new THREE.Float32BufferAttribute(grid, 3)), new THREE.LineBasicMaterial({ transparent: true, opacity: 0.7 })), '--plate-grid');
      gridLines.position.z = -0.1;
      bedGroup.add(gridLines);
      for (const r of bed.exclusions) {
        const ex = colored(new THREE.Mesh(new THREE.ShapeGeometry(shapeOf(rectPoints(r))), new THREE.MeshBasicMaterial({ transparent: true, opacity: 0.75, depthWrite: false })), '--keepout');
        ex.position.z = 0.02;
        bedGroup.add(ex);
      }
      // Размерные линии вдоль передней и правой сторон.
      const [front, side] = bed.labels;
      const a = bed.area, dims = [
        a.x0, front.y, 0, a.x1, front.y, 0, a.x0, front.y - 4, 0, a.x0, front.y + 4, 0, a.x1, front.y - 4, 0, a.x1, front.y + 4, 0,
        side.x, a.y0, 0, side.x, a.y1, 0, side.x - 4, a.y0, 0, side.x + 4, a.y0, 0, side.x - 4, a.y1, 0, side.x + 4, a.y1, 0,
      ];
      bedGroup.add(colored(new THREE.LineSegments(new THREE.BufferGeometry().setAttribute('position', new THREE.Float32BufferAttribute(dims, 3)), new THREE.LineBasicMaterial()), '--fg-dim'));
      scene.add(bedGroup);
    };

    /** Текстура подложки зависит от --accent-2: перерисовывается при смене темы без перестройки меша. */
    const paintFootprints = () => {
      if (!footprints || !footprintSource) return;
      const material = footprints.material as THREE.MeshBasicMaterial;
      const fp = footprintTexture(footprintSource.polygons, footprintSource.evenOdd);
      material.map?.dispose();
      material.map = fp.texture;
      material.needsUpdate = true;
    };

    const disposeObjects = () => {
      for (const o of [solid, cap, footprints]) if (o) { scene.remove(o); o.dispose(); }
      if (footprints) {
        footprints.geometry.dispose();
        const material = footprints.material as THREE.MeshBasicMaterial;
        material.map?.dispose();
        material.dispose();
      }
      modelGeometry?.dispose(); // общая для solid и cap; материалы общие на всю сцену
      modelGeometry = null;
      solid = cap = footprints = null;
      footprintSource = null;
    };

    const buildObjects = () => {
      disposeObjects();
      const { model, placements = [], footprintLoops } = propsRef.current;
      if (!model || !bed) return;
      const geometry = new THREE.BufferGeometry().setAttribute('position', new THREE.BufferAttribute(model.positions, 3));
      geometry.computeVertexNormals();
      modelGeometry = geometry;
      height = Math.max(model.size[2], 0.2);
      const spots = placements.length ? placements : [{ x: (bed.usable.x0 + bed.usable.x1) / 2, y: (bed.usable.y0 + bed.usable.y1) / 2, angleDeg: 0 }];
      solid = new THREE.InstancedMesh(geometry, objectMaterial, spots.length);
      cap = new THREE.InstancedMesh(geometry, capMaterial, spots.length);
      const evenOdd = Boolean(footprintLoops?.length);
      const polygons: [number, number][][] = footprintLoops?.length
        ? footprintLoops
        : Array.from({ length: model.triangles }, (_, t) => [0, 1, 2].map((k) => [model.positions[t * 9 + k * 3], model.positions[t * 9 + k * 3 + 1]] as [number, number]));
      footprintSource = { polygons, evenOdd };
      const fp = footprintTexture(polygons, evenOdd);
      footprints = new THREE.InstancedMesh(new THREE.PlaneGeometry(fp.size[0], fp.size[1]), new THREE.MeshBasicMaterial({ map: fp.texture, transparent: true, depthWrite: false }), spots.length);
      const m = new THREE.Matrix4(), q = new THREE.Quaternion(), s = new THREE.Vector3(1, 1, 1), z = new THREE.Vector3(0, 0, 1);
      spots.forEach((p, i) => {
        q.setFromAxisAngle(z, (p.angleDeg * Math.PI) / 180);
        m.compose(new THREE.Vector3(p.x, p.y, 0), q, s);
        solid!.setMatrixAt(i, m);
        cap!.setMatrixAt(i, m);
        const offset = new THREE.Vector3(fp.center[0], fp.center[1], 0).applyQuaternion(q);
        m.compose(new THREE.Vector3(p.x + offset.x, p.y + offset.y, 0.06), q, s);
        footprints!.setMatrixAt(i, m);
      });
      scene.add(solid, cap, footprints);
    };

    const fit = () => {
      const w = host.clientWidth, h = host.clientHeight;
      if (!w || !h || !bed) return;
      renderer.setSize(w, h, false);
      const cx = (bed.area.x0 + bed.area.x1) / 2, cy = (bed.area.y0 + bed.area.y1) / 2;
      const half = Math.max(bed.area.x1 - bed.area.x0, bed.area.y1 - bed.area.y0) * 0.5, aspect = w / h;
      camera.left = -half * aspect; camera.right = half * aspect; camera.top = half; camera.bottom = -half;
      if (!controls.enabled) { camera.position.set(cx + 420, cy - 420, 360); controls.target.set(cx, cy, 0); }
      camera.lookAt(cx, cy, 0);
      camera.updateProjectionMatrix();
      controls.update();
    };

    const v = new THREE.Vector3();
    let frame = 0;
    const draw = () => {
      frame = 0;
      const p = propsRef.current;
      const orbit = p.mode === 'done';
      if (controls.enabled !== orbit) { controls.enabled = orbit; fit(); }
      const clipZ = p.mode === 'running' ? Math.max(LAYER_MM, p.clip * height) : 1e4;
      clipPlane.constant = clipZ;
      // Без среза изнанка не нужна: иначе её край просвечивает оранжевым по контуру модели.
      if (cap) cap.visible = clipZ < height;
      slicePlane.visible = p.mode === 'running' && p.clip < 1 && bed !== null;
      if (bed && slicePlane.visible) {
        const a = bed.area;
        slicePlane.scale.set(a.x1 - a.x0 + 16, a.y1 - a.y0 + 16, 1);
        slicePlane.position.set((a.x0 + a.x1) / 2, (a.y0 + a.y1) / 2, clipZ);
      }
      renderer.render(scene, camera);
      if (labelsRef.current && bed) {
        const w = host.clientWidth, h = host.clientHeight;
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
    };
    // Анимации своей нет: кадр — по смене пропсов, размера или вращению мышью.
    const invalidate = () => {
      if (!frame) frame = requestAnimationFrame(draw);
    };

    const rebuild = (themeKey: string) => {
      const p = propsRef.current;
      const profileChanged = p.profile !== built.profile;
      if (profileChanged) buildBed(p.profile);
      const key = JSON.stringify(p.placements ?? []);
      const objectsChanged = profileChanged || p.model !== built.model || key !== built.placements || p.footprintLoops !== built.loops;
      if (objectsChanged) buildObjects();
      else if (themeKey !== built.theme) paintFootprints();
      built = { profile: p.profile, model: p.model, placements: key, loops: p.footprintLoops, theme: themeKey };
      paint();
      fit();
      invalidate();
    };
    apiRef.current = { rebuild, invalidate };
    const observer = new ResizeObserver(() => { fit(); invalidate(); });
    observer.observe(host);
    controls.addEventListener('change', invalidate);

    return () => {
      apiRef.current = null;
      cancelAnimationFrame(frame);
      observer.disconnect();
      controls.removeEventListener('change', invalidate);
      controls.dispose();
      disposeObjects();
      disposeTree(bedGroup);
      slicePlane.geometry.dispose();
      sliceMaterial.dispose();
      objectMaterial.dispose();
      capMaterial.dispose();
      renderer.dispose();
      renderer.domElement.remove();
    };
  }, []);

  // Перестройка при смене профиля/модели/раскладки и перекраска при смене темы
  // (тема уже применена к <html> до обновления стора — цвета токенов читаются свежими).
  useEffect(() => {
    apiRef.current?.rebuild(theme);
  }, [props.profile, props.model, props.placements, props.footprintLoops, theme]);

  return (
    <div className="scene" ref={hostRef}>
      <div className="scene-labels" ref={labelsRef} />
      <div className="scene-legend">
        <span>нельзя печатать <i className="swatch swatch-keepout" /></span>
        <span>контур проекции <i className="swatch swatch-footprint" /></span>
      </div>
      {props.caption && <div className="scene-tag scene-tag-bl">{props.caption}</div>}
      {props.readout && <div className="scene-tag scene-tag-br">{props.readout}</div>}
    </div>
  );
}
