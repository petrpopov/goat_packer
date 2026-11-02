import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { PRELIMINARY_HINT } from '@/utils/jobs';
import Icon from '@/components/Icon';
import * as THREE from 'three';
import { OrbitControls } from 'three/addons/controls/OrbitControls.js';
import type { PrinterProfile } from '@/api/types';
import { useUiStore } from '@/store/useUiStore';
import { bedSpec, type BedSpec, type Rect } from './bed';
import { bedGroup as buildBedGroup, towerMesh as buildTowerMesh } from './bedMeshes';
import { footprintTexture, type FootprintSource } from './footprintTexture';
import type { Placement } from './layout';
import type { ModelMesh } from './model3d';
import { projectTriangles } from './projection';
import { cssColor, degToRad, disposeTree, HIDDEN_Z, paintTokens, placementMatrix } from './three';

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
  /** Сколько объектов на столе — числом под легендой; null — раскладки ещё нет. */
  objectCount?: number | null;
  /** Число по промежуточной раскладке (итога ещё нет): приглушённое, с «≈». */
  countPreliminary?: boolean;
}

const LAYER_MM = 0.2;
/** Падение копий на стол после раскладки: общая длительность и высота старта, мм. */
const DROP_MS = 1400;
const DROP_FROM_MM = 120;
/** Перебор раскладок: сколько проекций мелькает, шаг смены кадра и разброс вокруг места, мм. */
const GHOSTS = 48;
const GHOST_TICK_MS = 200;
const GHOST_JITTER_MM = 6;
/** Проекции перебора: доля шага сетки от размера подложки и прозрачность. */
const GHOST_CELL = 0.9;
const GHOST_OPACITY = 0.7;
/** Высоты подложек и проекций над столом, мм (над башней). */
const FOOTPRINT_Z = 0.06;
const GHOST_Z = 0.07;
/** Камера: near/far и масштаб колесом; исходная точка обзора — сдвиг от центра стола, мм. */
const CAMERA_NEAR = 1;
const CAMERA_FAR = 2000;
const ZOOM_RANGE = { min: 0.7, max: 12 };
const VIEW_OFFSET = { x: 420, y: -420, z: 360 };
const MAX_PIXEL_RATIO = 2;
/** Свет: небо/земля полусферы и «солнце» сверху-спереди-слева. */
const HEMI_LIGHT = { sky: 0xffffff, ground: 0x666666, intensity: 1.6 };
const SUN_LIGHT = { color: 0xffffff, intensity: 1.5, position: [-150, -260, 420] as const };
/** Полупрозрачная плоскость среза слоя и её поле за краем стола, мм. */
const SLICE_OPACITY = 0.06;
const SLICE_MARGIN_MM = 16;
/** Без среза: плоскость отсечения далеко над моделью. */
const NO_CLIP_Z = 1e4;
/** Падение: каждая копия падает за долю общего времени, старты разнесены на остаток. */
const DROP_SHARE = 0.4;

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
    renderer.setPixelRatio(Math.min(window.devicePixelRatio, MAX_PIXEL_RATIO));
    renderer.localClippingEnabled = true;
    host.prepend(renderer.domElement);

    const scene = new THREE.Scene();
    // near/far 1..2000: шире — z-fighting слоёв стола (они в десятых долях мм друг от друга).
    const camera = new THREE.OrthographicCamera(-1, 1, 1, -1, CAMERA_NEAR, CAMERA_FAR);
    camera.up.set(0, 0, 1);
    scene.add(new THREE.HemisphereLight(HEMI_LIGHT.sky, HEMI_LIGHT.ground, HEMI_LIGHT.intensity));
    const sun = new THREE.DirectionalLight(SUN_LIGHT.color, SUN_LIGHT.intensity);
    sun.position.set(...SUN_LIGHT.position);
    scene.add(sun);
    // Вращение, масштаб колесом (к курсору — рассмотреть угол стола) и сдвиг правой кнопкой — всегда,
    // а не только на готовом столе: идущую нарезку тоже хочется разглядеть.
    const controls = new OrbitControls(camera, renderer.domElement);
    controls.zoomToCursor = true;
    controls.minZoom = ZOOM_RANGE.min;
    controls.maxZoom = ZOOM_RANGE.max;
    /** Камера в исходной точке обзора; false — поставить заново (новый стол, «Сбросить вид»). */
    let viewSet = false;

    const clipPlane = new THREE.Plane(new THREE.Vector3(0, 0, -1), NO_CLIP_Z);
    const objectMaterial = new THREE.MeshLambertMaterial({ clippingPlanes: [clipPlane] });
    // Внутренние грани, видимые сквозь срез, — сигнальным цветом: «горячий» срез слоя.
    const capMaterial = new THREE.MeshBasicMaterial({ side: THREE.BackSide, clippingPlanes: [clipPlane] });
    const sliceMaterial = new THREE.MeshBasicMaterial({
      transparent: true,
      opacity: SLICE_OPACITY,
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

    const paint = () => {
      paintTokens(bedGroup);
      if (towerMesh) (towerMesh.material as THREE.MeshBasicMaterial).color.copy(cssColor('--accent-2'));
      objectMaterial.color.copy(cssColor('--scene-object'));
      capMaterial.color.copy(cssColor('--signal'));
      sliceMaterial.color.copy(cssColor('--signal'));
    };

    const buildBed = (profile: PrinterProfile, usableOverride: Rect | null) => {
      scene.remove(bedGroup);
      disposeTree(bedGroup);
      const spec = bedSpec(profile);
      // Отступ от края пользователь может задать свой: реальная рабочая область — из раскладки.
      bed = usableOverride ? { ...spec, usable: usableOverride } : spec;
      bedGroup = buildBedGroup(bed);
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
      const m = new THREE.Matrix4();
      spots.forEach((p, i) => {
        // По порядку пакера — от центра наружу; каждая копия падает за DROP_SHARE общего времени.
        const start = (i / spots.length) * (1 - DROP_SHARE);
        const local = progress >= 1 ? 1 : Math.min(1, Math.max(0, (progress - start) / DROP_SHARE));
        const eased = 1 - (1 - local) ** 3;
        // Ещё не начавшая падать копия — за дальней плоскостью камеры.
        const z = progress >= 1 || local > 0 ? (1 - eased) * DROP_FROM_MM : HIDDEN_Z;
        placementMatrix(m, p.x, p.y, z, degToRad(p.angleDeg));
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
        towerMesh = buildTowerMesh(tower);
        scene.add(towerMesh);
      }
      if (!model || !bed) return;
      const geometry = new THREE.BufferGeometry().setAttribute(
        'position',
        new THREE.BufferAttribute(model.positions, 3)
      );
      geometry.computeVertexNormals();
      modelGeometry = geometry;
      height = Math.max(model.size[2], LAYER_MM);
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
      const m = new THREE.Matrix4();
      spots.forEach((p, i) => {
        // Контур — сразу на месте: копии падают на свои подложки.
        const angle = degToRad(p.angleDeg);
        const offset = new THREE.Vector3(fp.center[0], fp.center[1], 0).applyAxisAngle(
          new THREE.Vector3(0, 0, 1),
          angle
        );
        placementMatrix(m, p.x + offset.x, p.y + offset.y, FOOTPRINT_Z, angle);
        footprints!.setMatrixAt(i, m);
      });
      placeCopies(drop ? 0 : 1);
      dropStart = drop ? performance.now() : 0;
      scene.add(solid, cap, footprints);
      // Перебор раскладок (v2-макет): проекции модели по сетке рабочей области, кадр раз в GHOST_TICK_MS.
      const cell = Math.max(fp.size[0], fp.size[1]) * GHOST_CELL;
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
          opacity: GHOST_OPACITY,
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
      const m = new THREE.Matrix4();
      for (let i = 0; i < ghosts.count; i++) {
        const spot = ghostSpots[(i * 7) % ghostSpots.length];
        const on = (i * 7 + tick) % 4 === 0;
        placementMatrix(
          m,
          spot.x + Math.sin(tick + i) * GHOST_JITTER_MM,
          spot.y + Math.cos(tick * 2 + i) * GHOST_JITTER_MM,
          on ? GHOST_Z : HIDDEN_Z,
          ((tick + i) % 4) * 0.5
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
        camera.position.set(cx + VIEW_OFFSET.x, cy + VIEW_OFFSET.y, VIEW_OFFSET.z);
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
      const clipZ = p.mode === 'running' && !dropping && p.clip < 1 ? Math.max(LAYER_MM, p.clip * height) : NO_CLIP_Z;
      clipPlane.constant = clipZ;
      // Без среза изнанка не нужна: иначе её край просвечивает оранжевым по контуру модели.
      if (cap) cap.visible = !searching && clipZ < height;
      slicePlane.visible = p.mode === 'running' && !dropping && p.clip < 1 && bed !== null;
      if (bed && slicePlane.visible) {
        const a = bed.area;
        slicePlane.scale.set(a.x1 - a.x0 + SLICE_MARGIN_MM, a.y1 - a.y0 + SLICE_MARGIN_MM, 1);
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
          <b
            className={props.countPreliminary ? 'scene-count is-preliminary' : 'scene-count'}
            aria-label={`Объектов на столе: ${props.countPreliminary ? 'около ' : ''}${props.objectCount}`}
            title={props.countPreliminary ? PRELIMINARY_HINT : undefined}
          >
            {props.countPreliminary ? '≈ ' : ''}
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
