# Pack workspace and 3D bed scene — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Prerequisite:** `2026-11-01-webapp-shell-theme-login.md` is merged (routes, `useUiStore.navigate`, «Камень» tokens, `Icon`).

**Goal:** Turn «Упаковка» into a two-column workspace (vertical form left; action, staged progress, result and a three.js bed scene right), move AMS calibration into a side panel opened from the pack form, show «Учитывать реальную смену прутка» only inside the print-window option, and draw each printer's real bed (plate shape, dimensions, no-print zones) with the user's actual model previewed from the uploaded 3MF/STL.

**Architecture:** Pure, unit-tested modules do the work that can be tested in `bun`: `scene/model3d.ts` (STL/3MF → centred triangle soup, regex XML + `fflate` unzip, component transforms), `scene/bed.ts` (profile → plate outline, usable area, keepouts, dimension labels), `utils/progress.ts` (`stageSegments`, `sceneClip`). `scene/BedScene.tsx` is the only three.js code: orthographic isometric camera, extruded plate, keepout overlays, model mesh (instanced over `placements`, which stays empty until the `layout` event plan), projection + contour texture, clipping plane for slicing, orbit after success. It is lazy-loaded so three.js does not enter the main bundle. The backend exposes the plate geometry already present in `profiles/*.json` plus a new hand-drawn `plate_outline`.

**Tech Stack:** React 18 + TS, three.js (`three`, `@types/three`), `fflate`, plain CSS, `bun test`; Kotlin/Spring (`ProfileService`), JUnit 5.

**Design sources:** `DESIGN.md` (normative), mockup `docs/superpowers/specs/2026-10-31-webapp-redesign-mockup-v2.html` (scene behaviour, `Scene` module at the bottom of the file is the reference implementation), brief `docs/superpowers/specs/2026-10-31-webapp-redesign-brief.md` («v2» section items 4–6).

**Licensing rule:** slicer bed models/textures (Bambu Studio / Orca `bed_model` STL, `bed_texture` SVG) are AGPL-3.0 — never copy them. `plate_outline` below is our own simplified polygon drawn from measured dimensions.

---

## File structure

| File | Responsibility |
|---|---|
| `profiles/p1s.json`, `bambu_a2l.json`, `snapmaker_u1.json` | + `plate_outline` (mm, bed coords) |
| `webapp/backend/.../dto/JobDtos.kt` `ProfileDto` | + `printableArea`, `exclusionRectangles`, `plateOutline` |
| `webapp/backend/.../service/ProfileService.kt` | fill the new fields |
| `webapp/backend/src/test/.../profiles/ProfilesIntegrationTest.kt` | assert them |
| `webapp/frontend/src/api/types.ts` | `PrinterProfile` + the same fields |
| `src/scene/model3d.ts` + `tests/model3d.test.ts` | parse STL/3MF, centre on bed |
| `src/scene/bed.ts` + `tests/bed.test.ts` | bed spec from profile |
| `src/utils/progress.ts` + `tests/progress.test.ts` | `stageSegments`, `sceneClip` |
| `src/utils/forms.ts` + `tests/contracts.test.ts` | `packTimeFields` (AMS only with window) |
| `src/scene/BedScene.tsx` | three.js scene |
| `src/components/PackProgress.tsx` | segmented bar + stage chips |
| `src/components/AmsCorrectionFields.tsx` | + optional «откалибровать…» link |
| `src/pages/CalibrationPage.tsx` | `embedded` mode for the side panel |
| `src/pages/PackPage.tsx` | workspace layout |
| `src/pages/SchedulePage.tsx` | same two-column layout |
| `src/styles/workspace.css` | workspace, progress, scene card, side panel |
| `src/store/useUiStore.ts`, `src/App.tsx`, `src/components/Shell.tsx` | `calibOpen`; `/calibration` opens the panel; drop the sidebar item |

---

### Task 1: Plate outline in profiles and the profiles API

**Files:**
- Modify: `profiles/p1s.json`, `profiles/bambu_a2l.json`, `profiles/snapmaker_u1.json`
- Modify: `webapp/backend/src/main/kotlin/com/goatpacker/web/dto/JobDtos.kt` (`ProfileDto`)
- Modify: `webapp/backend/src/main/kotlin/com/goatpacker/web/service/ProfileService.kt` (`parseProfile`)
- Test: `webapp/backend/src/test/kotlin/com/goatpacker/web/profiles/ProfilesIntegrationTest.kt`

- [ ] **Step 1: Failing test.** In `ProfilesIntegrationTest.kt`, next to the existing `p1s` / `u1` assertions (they read the JSON node of each profile from `/api/profiles`), add:

```kotlin
        assertEquals(listOf(0.0, 0.0, 256.0, 256.0), p1s["printableArea"].map { it.asDouble() })
        assertEquals(listOf(listOf(0.0, 0.0, 18.0, 28.0)), p1s["exclusionRectangles"].map { r -> r.map { it.asDouble() } })
        assertTrue(p1s["plateOutline"].size() >= 8, "P1S plate outline is a polygon")
        assertEquals(listOf(0.5, 1.0, 270.5, 271.0), u1["printableArea"].map { it.asDouble() })
        assertTrue(u1["exclusionRectangles"].isEmpty)
```

Run (from `webapp/backend`): `./gradlew test --tests '*ProfilesIntegrationTest*'`
Expected: FAIL — `printableArea` missing.

- [ ] **Step 2: Profiles.** Add one key to each JSON (keep all other keys untouched; Rust `PrinterProfile` ignores unknown keys — no `deny_unknown_fields`):

`profiles/p1s.json`:
```json
  "plate_outline": [[7,0],[71,0],[81,-10],[251,-10],[258,-3],[258,251],[251,258],[163,258],[155,266],[103,266],[95,258],[7,258],[0,251],[0,7]],
```
`profiles/bambu_a2l.json`:
```json
  "plate_outline": [[2,-18.5],[108,-18.5],[124,-3],[205,-3],[221,-18.5],[328,-18.5],[332.5,-14],[332.5,325],[329.5,328],[301,328],[296,323],[35,323],[30,328],[0.5,328],[-2.5,325],[-2.5,-14]],
```
`profiles/snapmaker_u1.json`:
```json
  "plate_outline": [[4.5,-10.5],[266.5,-10.5],[275.5,-1.5],[275.5,273.5],[266.5,282.5],[4.5,282.5],[-4.5,273.5],[-4.5,-1.5]],
```

Verify Rust still loads them: from repo root `cargo test --release profile` → PASS.

- [ ] **Step 3: DTO.** In `ProfileDto` (JobDtos.kt) add after `edgeMarginMm`:

```kotlin
    /** printable_area профиля [x0, y0, x1, y1], мм, координаты стола. */
    val printableArea: List<Double> = emptyList(),
    /** exclusion_rectangles профиля — зоны, где печатать нельзя (у P1S угол 18×28). */
    val exclusionRectangles: List<List<Double>> = emptyList(),
    /** Упрощённый контур пластины для сцены; null — фронт рисует прямоугольник области. */
    val plateOutline: List<List<Double>>? = null,
```

- [ ] **Step 4: Fill them** in `ProfileService.parseProfile`, inside the `ProfileDto(...)` call:

```kotlin
                printableArea = area.map { it.asDouble() },
                exclusionRectangles = node.path("exclusion_rectangles")
                    .filter { r -> r.isArray && r.size() == 4 && r.all { it.isNumber } }
                    .map { r -> r.map { it.asDouble() } },
                plateOutline = node.get("plate_outline")
                    ?.takeIf { o -> o.isArray && o.size() >= 3 && o.all { p -> p.isArray && p.size() == 2 && p.all { it.isNumber } } }
                    ?.map { p -> p.map { it.asDouble() } },
```

- [ ] **Step 5: Run** `./gradlew test --tests '*ProfilesIntegrationTest*'` → PASS; then `./gradlew test` → all PASS.

- [ ] **Step 6: Frontend type.** In `webapp/frontend/src/api/types.ts`, `PrinterProfile`, add:

```ts
  /** [x0, y0, x1, y1], мм. */
  printableArea: number[];
  exclusionRectangles: number[][];
  /** Упрощённый контур пластины; null — прямоугольник области. */
  plateOutline: number[][] | null;
```

Fix any test fixture objects that build a `PrinterProfile` literal (`bun run typecheck` lists them) by adding `printableArea: [0, 0, 256, 256], exclusionRectangles: [], plateOutline: null`.

- [ ] **Step 7: Commit**

```bash
git add profiles webapp/backend webapp/frontend/src/api/types.ts webapp/frontend/tests
git commit -m "Profiles: plate outline; profiles API exposes printable area, exclusions, plate outline"
```

---

### Task 2: Dependencies

- [ ] **Step 1:** from `webapp/frontend`: `bun add three fflate && bun add -d @types/three`
- [ ] **Step 2:** `bun run typecheck && bun run build` → pass.
- [ ] **Step 3: Commit** — `git add package.json bun.lock && git commit -m "Webapp: three.js and fflate"`

---

### Task 3: Model parsing (pure, TDD)

**Files:**
- Create: `webapp/frontend/src/scene/model3d.ts`
- Test: `webapp/frontend/tests/model3d.test.ts`

- [ ] **Step 1: Failing tests** (all data generated in the test — no fixture paths outside `webapp/frontend`)

```ts
import { describe, expect, test } from 'bun:test';
import { strToU8, zipSync } from 'fflate';
import { parseModel } from '../src/scene/model3d';

/** Binary STL from triangles (each 9 numbers). */
function binaryStl(tris: number[][]): ArrayBuffer {
  const buf = new ArrayBuffer(84 + tris.length * 50);
  const dv = new DataView(buf);
  dv.setUint32(80, tris.length, true);
  tris.forEach((t, i) => t.forEach((v, k) => dv.setFloat32(84 + i * 50 + 12 + k * 4, v, true)));
  return buf;
}
// Тетраэдр 10×20×5 мм со смещением — проверяем центрирование.
const TET = [
  [100, 100, 7, 110, 100, 7, 100, 120, 7],
  [100, 100, 7, 100, 120, 7, 100, 100, 12],
  [100, 100, 7, 100, 100, 12, 110, 100, 7],
  [110, 100, 7, 100, 100, 12, 100, 120, 7],
];

describe('parseModel', () => {
  test('binary STL is centred: XY bbox centre at 0, min z at 0', () => {
    const m = parseModel('part.stl', binaryStl(TET));
    expect(m.triangles).toBe(4);
    expect(m.size.map((v) => Math.round(v * 1000) / 1000)).toEqual([10, 20, 5]);
    let minX = Infinity, maxX = -Infinity, minZ = Infinity;
    for (let i = 0; i < m.positions.length; i += 3) {
      minX = Math.min(minX, m.positions[i]); maxX = Math.max(maxX, m.positions[i]); minZ = Math.min(minZ, m.positions[i + 2]);
    }
    expect(minX).toBeCloseTo(-5); expect(maxX).toBeCloseTo(5); expect(minZ).toBeCloseTo(0);
  });

  test('ASCII STL', () => {
    const text = `solid t\n${TET.map((t) => `facet normal 0 0 0\nouter loop\n${[0, 3, 6].map((o) => `vertex ${t[o]} ${t[o + 1]} ${t[o + 2]}`).join('\n')}\nendloop\nendfacet`).join('\n')}\nendsolid t\n`;
    const m = parseModel('part.stl', strToU8(text).buffer as ArrayBuffer);
    expect(m.triangles).toBe(4);
    expect(m.size[1]).toBeCloseTo(20);
  });

  test('3MF follows build item → component (p:path) → mesh with transforms', () => {
    const mesh = `<model xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02"><resources><object id="1" type="model"><mesh><vertices>
      <vertex x="0" y="0" z="0"/><vertex x="4" y="0" z="0"/><vertex x="0" y="6" z="0"/><vertex x="0" y="0" z="3"/>
      </vertices><triangles><triangle v1="0" v2="1" v3="2"/><triangle v1="0" v2="2" v3="3"/><triangle v1="0" v2="3" v3="1"/><triangle v1="1" v2="3" v3="2"/></triangles></mesh></object></resources></model>`;
    // Компонент поворачивает на 90° вокруг Z: размеры X/Y меняются местами.
    const root = `<model xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02" xmlns:p="http://schemas.microsoft.com/3dmanufacturing/production/2015/06"><resources>
      <object id="2" type="model"><components><component objectid="1" p:path="/3D/Objects/mesh.model" transform="0 1 0 -1 0 0 0 0 1 10 20 -3"/></components></object>
      </resources><build><item objectid="2" transform="1 0 0 0 1 0 0 0 1 128 128 3"/></build></model>`;
    const zip = zipSync({ '3D/3dmodel.model': strToU8(root), '3D/Objects/mesh.model': strToU8(mesh) });
    const m = parseModel('project.3mf', zip.buffer as ArrayBuffer);
    expect(m.triangles).toBe(4);
    expect(m.size.map((v) => Math.round(v))).toEqual([6, 4, 3]);
  });

  test('unsupported extension throws a Russian message', () => {
    expect(() => parseModel('model.obj', new ArrayBuffer(10))).toThrow('Поддерживаются .3mf и .stl');
  });
});
```

- [ ] **Step 2: Run** — `bun test tests/model3d.test.ts` → FAIL (module missing).

- [ ] **Step 3: Implement `src/scene/model3d.ts`**

```ts
import { strFromU8, unzipSync } from 'fflate';

/** Треугольники модели (по 9 чисел), XY-центр габарита в 0, низ на z = 0 — как кладёт пакер. */
export interface ModelMesh {
  positions: Float32Array;
  triangles: number;
  /** Габарит, мм: [x, y, z]. */
  size: [number, number, number];
}

export function parseModel(name: string, buffer: ArrayBuffer): ModelMesh {
  const lower = name.toLowerCase();
  if (lower.endsWith('.stl')) return centred(parseStl(buffer));
  if (lower.endsWith('.3mf')) return centred(parse3mf(buffer));
  throw new Error('Поддерживаются .3mf и .stl');
}

function parseStl(buffer: ArrayBuffer): Float32Array {
  const dv = new DataView(buffer);
  if (buffer.byteLength >= 84) {
    const count = dv.getUint32(80, true);
    if (84 + count * 50 === buffer.byteLength) {
      const out = new Float32Array(count * 9);
      for (let i = 0; i < count; i++) for (let k = 0; k < 9; k++) out[i * 9 + k] = dv.getFloat32(84 + i * 50 + 12 + k * 4, true);
      return out;
    }
  }
  const text = new TextDecoder().decode(buffer);
  const values: number[] = [];
  for (const m of text.matchAll(/vertex\s+(\S+)\s+(\S+)\s+(\S+)/g)) values.push(+m[1], +m[2], +m[3]);
  return Float32Array.from(values);
}

type Matrix = number[]; // 3MF: 12 чисел, строка-вектор: p' = [x y z 1] · M
const IDENTITY: Matrix = [1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0];

function parseMatrix(text: string | undefined): Matrix {
  const v = (text ?? '').trim().split(/\s+/).map(Number);
  return v.length === 12 && v.every(Number.isFinite) ? v : IDENTITY;
}
function multiply(a: Matrix, b: Matrix): Matrix {
  // a затем b (оба в строковой записи 3MF)
  const r: Matrix = new Array(12).fill(0);
  for (let row = 0; row < 4; row++)
    for (let col = 0; col < 3; col++) {
      let s = row === 3 ? b[9 + col] : 0;
      for (let k = 0; k < 3; k++) s += a[row * 3 + k] * b[k * 3 + col];
      r[row * 3 + col] = s;
    }
  return r;
}
function attrs(tag: string): Record<string, string> {
  const out: Record<string, string> = {};
  for (const m of tag.matchAll(/([\w:]+)="([^"]*)"/g)) out[m[1]] = m[2];
  return out;
}

interface MeshObject { vertices: number[]; triangles: number[] }
interface ComponentRef { objectId: string; path: string | null; transform: Matrix }
interface ModelFile { objects: Map<string, MeshObject | ComponentRef[]>; items: { objectId: string }[] }

function parseModelXml(xml: string): ModelFile {
  const objects = new Map<string, MeshObject | ComponentRef[]>();
  for (const m of xml.matchAll(/<object\b([^>]*)>([\s\S]*?)<\/object>/g)) {
    const id = attrs(m[1]).id;
    const body = m[2];
    if (body.includes('<mesh')) {
      const vertices: number[] = [];
      for (const v of body.matchAll(/<vertex\b([^>]*)\/?>/g)) {
        const a = attrs(v[1]);
        vertices.push(+a.x, +a.y, +a.z);
      }
      const triangles: number[] = [];
      for (const t of body.matchAll(/<triangle\b([^>]*)\/?>/g)) {
        const a = attrs(t[1]);
        triangles.push(+a.v1, +a.v2, +a.v3);
      }
      objects.set(id, { vertices, triangles });
    } else {
      const refs: ComponentRef[] = [];
      for (const c of body.matchAll(/<component\b([^>]*)\/?>/g)) {
        const a = attrs(c[1]);
        refs.push({ objectId: a.objectid, path: a['p:path'] ?? null, transform: parseMatrix(a.transform) });
      }
      objects.set(id, refs);
    }
  }
  const items = [...xml.matchAll(/<item\b([^>]*)\/?>/g)].map((m) => ({ objectId: attrs(m[1]).objectid }));
  return { objects, items };
}

/**
 * Первый объект сборки со всеми компонентами (цветные части лежат в 3D/Objects/*.model).
 * Трансформ самого build item не применяется: модель всё равно центрируется на столе.
 */
function parse3mf(buffer: ArrayBuffer): Float32Array {
  const files = unzipSync(new Uint8Array(buffer), { filter: (f) => f.name.toLowerCase().endsWith('.model') });
  const models = new Map<string, ModelFile>();
  for (const [path, data] of Object.entries(files)) models.set('/' + path.replace(/^\/+/, ''), parseModelXml(strFromU8(data)));
  const rootPath = models.has('/3D/3dmodel.model') ? '/3D/3dmodel.model' : [...models.keys()].find((p) => models.get(p)!.items.length) ?? '';
  const root = models.get(rootPath);
  const first = root?.items[0];
  if (!root || !first) throw new Error('В 3MF не найден объект для печати');

  const out: number[] = [];
  const emit = (path: string, objectId: string, m: Matrix, depth: number) => {
    const object = models.get(path)?.objects.get(objectId);
    if (!object || depth > 8) return;
    if (Array.isArray(object)) {
      for (const ref of object) emit(ref.path ?? path, ref.objectId, multiply(ref.transform, m), depth + 1);
      return;
    }
    const v = object.vertices;
    for (const index of object.triangles) {
      const x = v[index * 3], y = v[index * 3 + 1], z = v[index * 3 + 2];
      out.push(x * m[0] + y * m[3] + z * m[6] + m[9], x * m[1] + y * m[4] + z * m[7] + m[10], x * m[2] + y * m[5] + z * m[8] + m[11]);
    }
  };
  emit(rootPath, first.objectId, IDENTITY, 0);
  if (!out.length) throw new Error('В 3MF нет треугольников');
  return Float32Array.from(out);
}

function centred(positions: Float32Array): ModelMesh {
  let minX = Infinity, minY = Infinity, minZ = Infinity, maxX = -Infinity, maxY = -Infinity, maxZ = -Infinity;
  for (let i = 0; i < positions.length; i += 3) {
    minX = Math.min(minX, positions[i]); maxX = Math.max(maxX, positions[i]);
    minY = Math.min(minY, positions[i + 1]); maxY = Math.max(maxY, positions[i + 1]);
    minZ = Math.min(minZ, positions[i + 2]); maxZ = Math.max(maxZ, positions[i + 2]);
  }
  const cx = (minX + maxX) / 2, cy = (minY + maxY) / 2;
  for (let i = 0; i < positions.length; i += 3) {
    positions[i] -= cx; positions[i + 1] -= cy; positions[i + 2] -= minZ;
  }
  return { positions, triangles: positions.length / 9, size: [maxX - minX, maxY - minY, maxZ - minZ] };
}
```

- [ ] **Step 4: Run** — `bun test tests/model3d.test.ts` → PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add src/scene/model3d.ts tests/model3d.test.ts
git commit -m "Webapp: parse STL/3MF in the browser for the bed preview"
```

---

### Task 4: Bed spec (pure, TDD)

**Files:**
- Create: `webapp/frontend/src/scene/bed.ts`
- Test: `webapp/frontend/tests/bed.test.ts`

- [ ] **Step 1: Failing tests**

```ts
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
```

- [ ] **Step 2: Run** — `bun test tests/bed.test.ts` → FAIL.

- [ ] **Step 3: Implement `src/scene/bed.ts`**

```ts
import type { PrinterProfile } from '../api/types';

export interface Rect { x0: number; y0: number; x1: number; y1: number }
export interface BedLabel { x: number; y: number; text: string }
export interface BedSpec {
  /** Контур пластины, мм, координаты стола (y — к задней стороне). */
  plate: [number, number][];
  /** printable_area профиля. */
  area: Rect;
  /** Область минус отступ от края — где реально может стоять объект. */
  usable: Rect;
  exclusions: Rect[];
  /** Подписи размеров и зон, z = 0. */
  labels: BedLabel[];
}

const DIM_OFFSET = 22; // мм от края пластины до размерной линии
const fmt = (v: number) => String(Math.round(v * 10) / 10);

export function bedSpec(profile: PrinterProfile): BedSpec {
  const [x0, y0, x1, y1] = profile.printableArea;
  const area = { x0, y0, x1, y1 };
  const m = profile.edgeMarginMm;
  const usable = { x0: x0 + m, y0: y0 + m, x1: x1 - m, y1: y1 - m };
  const plate: [number, number][] = profile.plateOutline?.length
    ? profile.plateOutline.map(([x, y]) => [x, y])
    : [[x0 - 3, y0 - 3], [x1 + 3, y0 - 3], [x1 + 3, y1 + 3], [x0 - 3, y1 + 3]];
  const exclusions = profile.exclusionRectangles.map(([a, b, c, d]) => ({ x0: a, y0: b, x1: c, y1: d }));
  const minY = Math.min(...plate.map((p) => p[1]));
  const maxX = Math.max(...plate.map((p) => p[0]));
  const labels: BedLabel[] = [
    { x: (x0 + x1) / 2, y: minY - DIM_OFFSET, text: `${fmt(x1 - x0)} мм` },
    { x: maxX + DIM_OFFSET, y: (y0 + y1) / 2, text: `${fmt(y1 - y0)} мм` },
    ...(m > 0 ? [{ x: x0 + m + 22, y: y1 - m / 2, text: `отступ ${fmt(m)} мм` }] : []),
    ...exclusions.map((r) => ({ x: r.x1 + 20, y: (r.y0 + r.y1) / 2, text: `${fmt(r.x1 - r.x0)}×${fmt(r.y1 - r.y0)}` })),
  ];
  return { plate, area, usable, exclusions, labels };
}
```

- [ ] **Step 4: Run** — `bun test tests/bed.test.ts` → PASS.
- [ ] **Step 5: Commit** — `git add src/scene/bed.ts tests/bed.test.ts && git commit -m "Webapp: bed spec from printer profile"`

---

### Task 5: Progress segments and slicing clip (pure, TDD)

**Files:**
- Modify: `webapp/frontend/src/utils/progress.ts`
- Test: `webapp/frontend/tests/progress.test.ts` (append)

Bands are the backend's (`webapp/README.md`, «progressPercent»): probe 1–15, packing 15–30, assembling 30–33, final slice 33–78, G-code check 78–82, repack 82–86, window 86–95, writing 95–99.

- [ ] **Step 1: Append failing tests** to `tests/progress.test.ts` (reuse the file's `stage(partial)` helper):

```ts
import { sceneClip, stageSegments } from '../src/utils/progress';

describe('stageSegments', () => {
  test('weights follow the backend bands; current stage is filled by percent', () => {
    const segs = stageSegments(stage({ id: 'final_slice', visited: ['probe_slice', 'packing', 'assembling', 'final_slice'] }), 55.5);
    expect(segs.map((s) => [s.id, s.weight, s.state])).toEqual([
      ['probe_slice', 14, 'done'], ['packing', 15, 'done'], ['assembling', 3, 'done'], ['final_slice', 45, 'current'],
      ['gcode_check', 4, 'pending'], ['repacking', 4, 'pending'], ['window_slice', 9, 'pending'], ['writing', 4, 'pending'],
    ]);
    expect(segs[3].fill).toBeCloseTo(0.5);
  });
  test('optional stages passed without a visit are skipped', () => {
    const segs = stageSegments(stage({ id: 'writing', visited: ['probe_slice', 'packing', 'assembling', 'final_slice', 'gcode_check'] }), 97);
    expect(segs.find((s) => s.id === 'repacking')!.state).toBe('skipped');
    expect(segs.find((s) => s.id === 'gcode_check')!.state).toBe('done');
  });
});

describe('sceneClip', () => {
  test('probe and final slice raise the cut plane through the stage band', () => {
    expect(sceneClip('probe_slice', 8)).toBeCloseTo(0.5);
    expect(sceneClip('final_slice', 33)).toBeCloseTo(0);
    expect(sceneClip('final_slice', 78)).toBeCloseTo(1);
  });
  test('other stages show whole objects', () => {
    expect(sceneClip('packing', 20)).toBe(1);
    expect(sceneClip('writing', 97)).toBe(1);
  });
});
```

- [ ] **Step 2: Run** — `bun test tests/progress.test.ts` → FAIL (exports missing).

- [ ] **Step 3: Implement** — append to `src/utils/progress.ts`:

```ts
/** Полосы процента этапов — те же, что у бэкенда (ProgressTracker.PackStage). */
const BANDS: Record<PackStageId, [number, number]> = {
  probe_slice: [1, 15],
  packing: [15, 30],
  assembling: [30, 33],
  final_slice: [33, 78],
  gcode_check: [78, 82],
  repacking: [82, 86],
  window_slice: [86, 95],
  writing: [95, 99],
};

export type SegmentState = 'done' | 'current' | 'pending' | 'skipped';
export interface StageSegment {
  id: PackStageId;
  label: string;
  /** Доля ширины полосы = ширина полосы этапа в процентах. */
  weight: number;
  state: SegmentState;
  /** Заполнение текущего сегмента 0..1. */
  fill: number;
}

/** Сегменты шкалы: все этапы по порядку; необязательные, пройденные без захода, — «skipped». */
export function stageSegments(stage: JobStage, percent: number): StageSegment[] {
  const visited = new Set(stage.visited);
  const currentIndex = ORDER.findIndex((step) => step.id === stage.id);
  return ORDER.map((step, index) => {
    const [start, end] = BANDS[step.id];
    let state: SegmentState;
    if (step.id === stage.id) state = 'current';
    else if (index < currentIndex) state = step.optional && !visited.has(step.id) ? 'skipped' : 'done';
    else state = 'pending';
    const fill = state === 'current' ? Math.min(1, Math.max(0, (percent - start) / (end - start))) : state === 'done' ? 1 : 0;
    return { id: step.id, label: step.label, weight: end - start, state, fill };
  });
}

/** Высота плоскости среза в сцене как доля высоты модели (1 — модель целиком). */
export function sceneClip(stageId: string, percent: number): number {
  if (stageId !== 'probe_slice' && stageId !== 'final_slice') return 1;
  const [start, end] = BANDS[stageId];
  return Math.min(1, Math.max(0, (percent - start) / (end - start)));
}
```

- [ ] **Step 4: Run** — `bun test tests/progress.test.ts` → PASS.
- [ ] **Step 5: Commit** — `git add src/utils/progress.ts tests/progress.test.ts && git commit -m "Webapp: stage segments and slicing clip from progress bands"`

---

### Task 6: AMS correction only inside the print window (pack form)

**Files:**
- Modify: `webapp/frontend/src/utils/forms.ts`
- Test: `webapp/frontend/tests/contracts.test.ts`

Schedule keeps `buildTimeFields` unchanged (there AMS correction matters without a window).

- [ ] **Step 1: Failing test** — append inside `describe('form contracts', …)` in `tests/contracts.test.ts` (import `packTimeFields` from `../src/utils/forms`):

```ts
  test('pack: AMS correction is sent only together with the print window', () => {
    expect(packTimeFields({ ...base, amsOn: true, changeSeconds: '80' }, true)).toEqual([]);
    expect(packTimeFields({ ...base, windowOn: true, amsOn: true, changeSeconds: '80' }, true)).toEqual([
      ['timeWindowHours', '12'],
      ['timeMarginPct', '10'],
      ['amsCorrection', 'true'],
      ['changeSeconds', '80'],
    ]);
  });
```

- [ ] **Step 2: Run** — `bun test tests/contracts.test.ts` → FAIL.

- [ ] **Step 3: Implement** — append to `src/utils/forms.ts`:

```ts
/**
 * Поля времени для упаковки: без окна стол заполняется максимально и время не влияет на
 * результат, поэтому коррекция смены прутка шлётся только вместе с окном печати.
 */
export function packTimeFields(state: TimeFormState, amsAvailable: boolean): [string, string][] {
  return buildTimeFields({ ...state, amsOn: state.windowOn && state.amsOn }, amsAvailable);
}
```

- [ ] **Step 4: Run** — PASS. **Step 5: Commit** — `git add src/utils/forms.ts tests/contracts.test.ts && git commit -m "Webapp: pack sends AMS correction only with a print window"`

---

### Task 7: BedScene (three.js)

**Files:**
- Create: `webapp/frontend/src/scene/BedScene.tsx`

The reference behaviour is the `Scene` module of the v2 mockup. Visual-only; verified by eye in Task 10.

- [ ] **Step 1: Write `src/scene/BedScene.tsx`**

```tsx
import { useEffect, useRef } from 'react';
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
  x.fillStyle = getComputedStyle(document.documentElement).getPropertyValue(name).trim() || '#888';
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

/** Подложка «проекция + контур»: заливка 28 % и обводка 1.6 мм цветом --accent-2. */
function footprintTexture(polygons: [number, number][][], evenOdd: boolean): { texture: THREE.CanvasTexture; size: [number, number]; center: [number, number] } {
  let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
  for (const poly of polygons) for (const [x, y] of poly) { minX = Math.min(minX, x); maxX = Math.max(maxX, x); minY = Math.min(minY, y); maxY = Math.max(maxY, y); }
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
  ox.globalCompositeOperation = 'source-over'; ox.globalAlpha = 0.28; ox.drawImage(ring, 0, 0);
  const texture = new THREE.CanvasTexture(out);
  texture.colorSpace = THREE.SRGBColorSpace;
  return { texture, size: [w / res, h / res], center: [(minX + maxX) / 2, (minY + maxY) / 2] };
}

export default function BedScene(props: BedSceneProps) {
  const hostRef = useRef<HTMLDivElement>(null);
  const labelsRef = useRef<HTMLDivElement>(null);
  const propsRef = useRef(props);
  propsRef.current = props;
  const theme = useUiStore((s) => s.theme);
  const apiRef = useRef<{ rebuild: () => void } | null>(null);

  useEffect(() => {
    const host = hostRef.current!;
    const renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true });
    renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
    renderer.localClippingEnabled = true;
    host.prepend(renderer.domElement);

    const scene = new THREE.Scene();
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
    let solid: THREE.InstancedMesh | null = null;
    let cap: THREE.InstancedMesh | null = null;
    let footprints: THREE.InstancedMesh | null = null;
    let height = 1;
    let built = { profile: null as PrinterProfile | null, model: null as ModelMesh | null, placements: '' , loops: null as unknown };

    const colored = (mesh: THREE.Mesh | THREE.LineSegments, token: string) => {
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

    const buildObjects = () => {
      for (const o of [solid, cap, footprints]) if (o) { scene.remove(o); o.dispose(); }
      solid = cap = footprints = null;
      const { model, placements = [], footprintLoops } = propsRef.current;
      if (!model || !bed) return;
      const geometry = new THREE.BufferGeometry().setAttribute('position', new THREE.BufferAttribute(model.positions, 3));
      geometry.computeVertexNormals();
      height = Math.max(model.size[2], 0.2);
      const spots = placements.length ? placements : [{ x: (bed.usable.x0 + bed.usable.x1) / 2, y: (bed.usable.y0 + bed.usable.y1) / 2, angleDeg: 0 }];
      solid = new THREE.InstancedMesh(geometry, objectMaterial, spots.length);
      cap = new THREE.InstancedMesh(geometry, capMaterial, spots.length);
      const polygons: [number, number][][] = footprintLoops?.length
        ? footprintLoops
        : Array.from({ length: model.triangles }, (_, t) => [0, 1, 2].map((k) => [model.positions[t * 9 + k * 3], model.positions[t * 9 + k * 3 + 1]] as [number, number]));
      const fp = footprintTexture(polygons, Boolean(footprintLoops?.length));
      footprints = new THREE.InstancedMesh(new THREE.PlaneGeometry(fp.size[0], fp.size[1]), new THREE.MeshBasicMaterial({ map: fp.texture, transparent: true, depthWrite: false }), spots.length);
      const m = new THREE.Matrix4(), q = new THREE.Quaternion(), s = new THREE.Vector3(1, 1, 1), z = new THREE.Vector3(0, 0, 1);
      spots.forEach((p, i) => {
        q.setFromAxisAngle(z, (p.angleDeg * Math.PI) / 180);
        m.compose(new THREE.Vector3(p.x, p.y, 0), q, s);
        solid!.setMatrixAt(i, m);
        cap!.setMatrixAt(i, m);
        const offset = new THREE.Vector3(fp.center[0], fp.center[1], 0.06).applyQuaternion(q);
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

    const rebuild = () => {
      const p = propsRef.current;
      if (p.profile !== built.profile) buildBed(p.profile);
      const key = JSON.stringify(p.placements ?? []);
      if (p.profile !== built.profile || p.model !== built.model || key !== built.placements || p.footprintLoops !== built.loops) buildObjects();
      built = { profile: p.profile, model: p.model, placements: key, loops: p.footprintLoops };
      paint();
      fit();
    };
    apiRef.current = { rebuild };
    rebuild();
    const observer = new ResizeObserver(fit);
    observer.observe(host);

    const reduced = window.matchMedia('(prefers-reduced-motion: reduce)');
    const v = new THREE.Vector3();
    let frame = 0;
    const tick = () => {
      const p = propsRef.current;
      const orbit = p.mode === 'done';
      if (controls.enabled !== orbit) { controls.enabled = orbit; fit(); }
      const clipZ = p.mode === 'running' ? Math.max(LAYER_MM, p.clip * height) : 1e4;
      clipPlane.constant = clipZ;
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
      frame = reduced.matches && p.mode !== 'done' ? 0 : requestAnimationFrame(tick);
    };
    tick();

    return () => {
      cancelAnimationFrame(frame);
      observer.disconnect();
      controls.dispose();
      renderer.dispose();
      renderer.domElement.remove();
    };
  }, []);

  // Перестройка при смене профиля/модели/раскладки и перекраска при смене темы.
  useEffect(() => {
    apiRef.current?.rebuild();
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
```

- [ ] **Step 2:** `bun run typecheck && bun run lint` → pass (fix lint findings in this file only).
- [ ] **Step 3: Commit** — `git add src/scene/BedScene.tsx && git commit -m "Webapp: three.js bed scene (profile bed, model, projection, slicing clip)"`

---

### Task 8: Workspace CSS, segmented progress, AMS link, calibration panel

**Files:**
- Create: `webapp/frontend/src/styles/workspace.css` (import in `main.tsx` after `shell.css`)
- Rewrite: `webapp/frontend/src/components/PackProgress.tsx`
- Modify: `webapp/frontend/src/components/AmsCorrectionFields.tsx`, `webapp/frontend/src/pages/CalibrationPage.tsx`, `webapp/frontend/src/store/useUiStore.ts`

- [ ] **Step 1: `src/styles/workspace.css`** (from the mockup)

```css
.work { display: grid; grid-template-columns: 320px minmax(0, 1fr); min-height: calc(100vh - 52px); }
.work.with-side { grid-template-columns: 320px minmax(0, 1fr) 340px; }
.form-col { display: grid; gap: 16px; align-content: start; padding: 18px 18px 28px; border-right: 1px solid var(--border); }
.form-group { display: grid; gap: 12px; padding-top: 14px; border-top: 1px solid var(--border); }
.nested { display: grid; gap: 12px; margin: 2px 0 0 24px; }
.main-col { display: grid; gap: 16px; align-content: start; min-width: 0; padding: 18px 24px 28px; }
.side-col { display: grid; gap: 16px; align-content: start; padding: 18px; border-left: 1px solid var(--border); }
.side-head { display: flex; align-items: center; justify-content: space-between; }
.side-head h2 { margin: 0; font-size: 15px; font-weight: 600; }
.action-bar { display: flex; align-items: center; gap: 16px; }
.action-bar .title { flex: 1; min-width: 0; }
.action-bar h1 { margin: 0; font-size: 18px; font-weight: 600; letter-spacing: -0.015em; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.action-bar p { margin: 2px 0 0; color: var(--fg-dim); }
.seg { display: grid; grid-auto-flow: column; grid-auto-columns: 1fr; padding: 2px; background: var(--panel-2); border: 1px solid var(--border); border-radius: var(--radius); }
.seg button { height: 26px; border: 0; border-radius: var(--radius-sm); background: transparent; color: var(--fg-muted); font: 500 12.5px var(--font-ui); cursor: pointer; }
.seg button[aria-pressed='true'] { background: var(--panel); color: var(--fg); box-shadow: var(--shadow-sm), 0 0 0 1px var(--border); }

/* Прогресс: сегменты пропорциональны полосам этапов */
.stage-bar { display: flex; gap: 2px; height: 8px; overflow: hidden; border-radius: 4px; background: var(--panel-2); }
.stage-bar span { height: 100%; background: var(--border-strong); opacity: 0.35; }
.stage-bar span.done { background: var(--signal); opacity: 1; }
.stage-bar span.current { opacity: 1; background: linear-gradient(90deg, var(--signal) var(--fill), var(--border-strong) var(--fill)); }
.stage-bar span.skipped { background: repeating-linear-gradient(135deg, var(--border-strong) 0 3px, transparent 3px 6px); opacity: 0.5; }
.stage-chips { display: flex; flex-wrap: wrap; gap: 4px 14px; margin: 0; padding: 0; list-style: none; color: var(--fg-dim); font-size: 12px; }
.stage-chips li { display: inline-flex; align-items: center; gap: 5px; }
.stage-chips i { width: 7px; height: 7px; border-radius: 50%; background: var(--border-strong); }
.stage-chips .done { color: var(--fg-muted); }
.stage-chips .done i { background: var(--signal); }
.stage-chips .current { color: var(--fg); font-weight: 500; }
.stage-chips .current i { background: var(--signal); box-shadow: 0 0 0 3px var(--accent-soft); }
.stage-chips .skipped i, .stage-chips .optional i { background: transparent; border: 1px dashed var(--border-strong); }

/* Сцена */
.scene { position: relative; aspect-ratio: 16 / 9.4; overflow: hidden; border: 1px solid var(--border); border-radius: var(--radius-lg); background: var(--scene-bg); }
.scene canvas { display: block; width: 100% !important; height: 100% !important; }
.scene-labels { position: absolute; inset: 0; pointer-events: none; }
.scene-dim { position: absolute; transform: translate(-50%, -50%); padding: 1px 5px; border-radius: 3px; background: var(--scene-bg); color: var(--fg-muted); font: 500 11.5px var(--font-mono); white-space: nowrap; }
.scene-legend { position: absolute; top: 12px; right: 14px; display: grid; gap: 4px; color: var(--fg-dim); font-size: 12px; pointer-events: none; }
.scene-legend span { display: flex; align-items: center; justify-content: flex-end; gap: 7px; }
.swatch { width: 12px; height: 9px; border-radius: 2px; }
.swatch-keepout { background: var(--keepout); opacity: 0.8; }
.swatch-footprint { border: 2px solid var(--accent-2); }
.scene-tag { position: absolute; color: var(--fg-dim); font-size: 12px; pointer-events: none; }
.scene-tag-bl { left: 14px; bottom: 12px; }
.scene-tag-br { right: 14px; bottom: 12px; text-align: right; }

@media (max-width: 1180px) {
  .work, .work.with-side { grid-template-columns: 300px minmax(0, 1fr); }
  .side-col { grid-column: 1 / -1; border-left: 0; border-top: 1px solid var(--border); }
}
@media (max-width: 760px) {
  .work, .work.with-side { grid-template-columns: 1fr; }
  .form-col { border-right: 0; border-bottom: 1px solid var(--border); }
}
```

- [ ] **Step 2: Rewrite `src/components/PackProgress.tsx`**

```tsx
import { useEffect, useState } from 'react';
import type { JobStage } from '@/api/types';
import { looksLikeJson, stageElapsed, stageSegments, stageTimeLine } from '@/utils/progress';

interface PackProgressProps {
  percent: number;
  stage: JobStage;
  /** Русское описание этапа от бэкенда. */
  message: string | null;
  receivedAtMs?: number;
}

const TICK_MS = 1000;
const OPTIONAL = new Set(['gcode_check', 'repacking', 'window_slice']);

/** Одна шкала из сегментов этапов (ширина = полоса процента этапа) + строка статуса + этапы. */
export default function PackProgress({ percent, stage, message, receivedAtMs }: PackProgressProps) {
  const [now, setNow] = useState(receivedAtMs ?? 0);
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), TICK_MS);
    return () => window.clearInterval(timer);
  }, []);
  const elapsed = stageElapsed(stage, receivedAtMs, Math.max(now, receivedAtMs ?? 0));
  const segments = stageSegments(stage, percent);
  const status = message && !looksLikeJson(message) ? message : segments.find((s) => s.state === 'current')?.label;

  return (
    <div className="pack-progress" role="group" aria-label="Ход упаковки">
      <div className="stage-status">
        <b>{status}</b>
        <span className="stage-status-time mono">
          {Math.round(percent)}% · {stageTimeLine(elapsed, stage.expectedSeconds)}
        </span>
      </div>
      <div className="stage-bar" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(percent)}>
        {segments.map((s) => (
          <span key={s.id} className={s.state} style={{ flex: s.weight, ['--fill' as string]: `${Math.round(s.fill * 100)}%` }} />
        ))}
      </div>
      <ul className="stage-chips">
        {segments.map((s) => {
          const optional = OPTIONAL.has(s.id) && s.state === 'pending';
          return (
            <li key={s.id} className={optional ? 'optional' : s.state}>
              <i aria-hidden="true" />
              {s.label}
              {s.state === 'skipped' ? ' · не понадобилась' : optional ? ' · если понадобится' : ''}
            </li>
          );
        })}
      </ul>
    </div>
  );
}
```

Update `.stage-status` in `core.css` to `display: flex; justify-content: space-between; gap: 16px;` if it is not already a flex row. Delete the now-unused `.stage-step*`, `.stage-marker` blocks from `core.css`.

- [ ] **Step 3: AMS link.** In `AmsCorrectionFields.tsx` add an optional prop and render it under the seconds field:

```tsx
  /** Открыть панель калибровки (только в упаковке). */
  onCalibrate?: () => void;
```
```tsx
      {on && onCalibrate && (
        <button type="button" className="link" style={{ justifySelf: 'start' }} onClick={onCalibrate}>
          Откалибровать…
        </button>
      )}
```
Rename the checkbox label to «Учитывать реальную смену прутка» and the hint to `слайсер занижает время смены AMS — окно считается точнее` (keep `для этого принтера коррекция не нужна` when unavailable).

- [ ] **Step 4: Store.** In `useUiStore.ts` add `calibOpen: boolean` (initial `false`) and `setCalibOpen: (open: boolean) => void` (`set({ calibOpen: open })`).

- [ ] **Step 5: Calibration embedded.** In `CalibrationPage.tsx` change the signature to `export default function CalibrationPage({ embedded = false, onClose }: { embedded?: boolean; onClose?: () => void })`. When `embedded`:
  - replace the `<h1 className="page-title">` + subtitle block with
    ```tsx
    <div className="side-head">
      <h2>Калибровка смены прутка</h2>
      <button type="button" className="icon-btn" onClick={onClose} aria-label="Закрыть"><Icon name="x" /></button>
    </div>
    ```
  - the outer wrapper is a fragment-like `<div className="stack">` instead of `<div className="page">`, and `.form-grid` inside becomes a single column (add `className={embedded ? 'stack' : 'form-grid'}`).
  The calculator already writes the result to `useCalibrationStore`, which `useAmsCorrection` reads — the new value lands in the pack form without extra wiring.

- [ ] **Step 6: Verify** — `bun run typecheck && bun run lint && bun test tests` → pass.
- [ ] **Step 7: Commit** — `git add -A src && git commit -m "Webapp: workspace styles, segmented stage progress, calibration side panel"`

---

### Task 9: Pack and schedule pages in the workspace layout

**Files:**
- Modify: `webapp/frontend/src/pages/PackPage.tsx`, `webapp/frontend/src/pages/SchedulePage.tsx`, `webapp/frontend/src/App.tsx`, `webapp/frontend/src/components/Shell.tsx`

- [ ] **Step 1: PackPage state additions** (top of the component):

```tsx
  const calibOpen = useUiStore((s) => s.calibOpen);
  const setCalibOpen = useUiStore((s) => s.setCalibOpen);
  const [model, setModel] = useState<ModelMesh | null>(null);
  useEffect(() => {
    let cancelled = false;
    setModel(null);
    if (!file) return;
    file.arrayBuffer()
      .then((buffer) => !cancelled && setModel(parseModel(file.name, buffer)))
      .catch((e) => !cancelled && pushToast(e instanceof Error ? e.message : 'Не удалось прочитать модель', 'error'));
    return () => { cancelled = true; };
  }, [file]);
```
Imports: `import { lazy, Suspense } from 'react'`, `import { parseModel, type ModelMesh } from '@/scene/model3d'`, `import { packTimeFields } from '@/utils/forms'`, `import { sceneClip } from '@/utils/progress'`, `import Icon from '@/components/Icon'`, `import CalibrationPage from '@/pages/CalibrationPage'`, and at module level `const BedScene = lazy(() => import('@/scene/BedScene'));`.

Replace `buildTimeFields(` with `packTimeFields(` in the time-fields block. Close the calibration panel when the window option is switched off:

```tsx
  useEffect(() => {
    if (!windowOn) setCalibOpen(false);
  }, [windowOn, setCalibOpen]);
```

- [ ] **Step 2: Replace the whole `return (…)` of PackPage** with the workspace layout. Every field keeps its current state/handlers; only placement and labels change:

```tsx
  const running = job?.status === 'RUNNING' || job?.status === 'FINALIZING';
  const sceneMode = running ? 'running' : job?.status === 'SUCCEEDED' ? 'done' : 'idle';
  const busy = jobActive || startMutation.isPending;

  return (
    <div className={`work${calibOpen ? ' with-side' : ''}`}>
      <form id="pack-form" className="form-col" onSubmit={handleSubmit}>
        <FileDrop file={file} onFileChange={handleFileChange} disabled={busy} />
        {detectionError ? <div className="error-block">{detectionError}</div> : null}
        <Field label="Профиль принтера" hint={detection === 'detecting' ? 'Определяю принтер по файлу…' : detection === 'ok' ? 'Определён по файлу' : undefined}>
          <div className="input" aria-readonly="true">{selectedProfile ? selectedProfile.name : '—'}</div>
        </Field>
        <Field label="Раскладка" hint={layout === 'grid' ? 'быстро и равномерно, без поворотов' : 'подбор с поворотами и нависаниями'}>
          <div className="seg" role="group" aria-label="Раскладка">
            <button type="button" aria-pressed={layout === 'search'} onClick={() => setLayout('search')} disabled={busy}>Плотная</button>
            <button type="button" aria-pressed={layout === 'grid'} onClick={() => setLayout('grid')} disabled={busy}>Сетка</button>
          </div>
        </Field>
        {/* Чистка сопла: перенести без изменений текущие Field «Чистка сопла» и условные поля
            «Каждый N слой» / «Высота H, мм» (блоки cleanMode === 'global' / 'from-z'). */}
        {nozzleCleanFields}
        <div className="form-group">
          <label className="check">
            <input type="checkbox" checked={windowOn} onChange={(e) => setWindowOn(e.target.checked)} disabled={busy} />
            <span>
              Уложиться в окно печати
              <small className="field-hint">без окна стол заполняется максимально</small>
            </span>
          </label>
          {windowOn && (
            <div className="nested">
              {windowFields}
              <AmsCorrectionFields
                available={ams.available}
                on={ams.on}
                onToggle={ams.setOn}
                seconds={ams.seconds}
                onSecondsChange={ams.setSeconds}
                invalid={ams.seconds.trim() !== '' && !numberInRange(ams.seconds, 0, 600)}
                disabled={busy}
                onCalibrate={() => setCalibOpen(!calibOpen)}
              />
            </div>
          )}
        </div>
        <details className="params form-group">
          <summary className="params-summary">Дополнительно</summary>
          {supportOverlapField}
          {paramsFields}
        </details>
      </form>

      <section className="main-col">
        <div className="action-bar">
          <div className="title">
            <h1>{file ? file.name : 'Модель не выбрана'}</h1>
            <p>
              {selectedProfile
                ? `${selectedProfile.name} · рабочая область ${selectedProfile.bedWidthMm} × ${selectedProfile.bedDepthMm} мм`
                : 'Загрузите слева проект .3mf с одним объектом'}
            </p>
          </div>
          {job?.status === 'SUCCEEDED' ? (
            <button type="submit" form="pack-form" className="btn" disabled={!canSubmit}>Упаковать заново</button>
          ) : (
            <button type="submit" form="pack-form" className="btn btn-primary btn-lg" disabled={!canSubmit}>
              {startMutation.isPending ? <span className="spinner" aria-hidden="true" /> : null}
              Упаковать
            </button>
          )}
        </div>
        <JobCard jobId={jobId} onDismiss={() => setJobId(null)} job={job} pollError={jobQuery.isError ? jobQuery.error : undefined} successContent={successContent} />
        {selectedProfile && (
          <Suspense fallback={<div className="scene" />}>
            <BedScene
              profile={selectedProfile}
              model={model}
              mode={sceneMode}
              clip={running && job?.stage ? sceneClip(job.stage.id, job.progressPercent) : 1}
              caption={sceneMode === 'done' ? 'перетащите, чтобы повернуть' : model ? 'модель и её проекция на стол' : ''}
              readout={model ? `${model.size.map((v) => Math.round(v)).join(' × ')} мм` : ''}
            />
          </Suspense>
        )}
      </section>

      {calibOpen && (
        <aside className="side-col">
          <CalibrationPage embedded onClose={() => setCalibOpen(false)} />
        </aside>
      )}
    </div>
  );
```

Before the `return`, extract the existing JSX into four constants, moving the code verbatim from the old return:
- `const nozzleCleanFields = (<>…</>)` — the «Чистка сопла» `Field` and the `cleanMode === 'global'` / `'from-z'` blocks;
- `const windowFields = (<div className="row2">…</div>)` — the two `Field`s «Окно печати, ч» and «Запас, %» (rename the first label to «Окно, ч»); add `.row2 { display: grid; grid-template-columns: 1fr 1fr; gap: 10px; }` to `workspace.css`;
- `const supportOverlapField = (…)` — the «Перекрытие поддержек, мм» `Field`;
- `const paramsFields = (<>…</>)` — the three `Field`s inside the old `<details className="params">` («Зазор», «Разрешение растра» → label «Растр, мм», «Отступ от края»).

`successContent`: replace `→ В расписание` text with `В расписание <Icon name="arrow-right" />` and keep the `onClick`.

The old JSX blocks for the window checkbox, `AmsCorrectionFields`, the `<details className="params">` wrapper, the `form-actions` submit row and the page title are deleted (they are replaced above).

- [ ] **Step 3: SchedulePage layout.** Wrap its return in the same skeleton: `<div className="work">` → `<form id="schedule-form" className="form-col" …>` containing the existing `card` content (source switch, file/handoff block, all `Field`s, `AmsCorrectionFields` without `onCalibrate`); `<section className="main-col">` with an `action-bar` (`<h1>Расписание на сутки</h1>`, subtitle = selected printer name, submit button `form="schedule-form"` labelled «Построить») followed by the existing `JobCard` and result tables. Delete the page title block and the old `form-actions` row.

- [ ] **Step 4: Remove the calibration section.**
  - `Shell.tsx`: delete the `/calibration` item from `WORK_ITEMS`.
  - `App.tsx`: delete the `<div hidden={visible !== 'calibration'}><CalibrationPage /></div>` block and its import; add before `return`:

```tsx
  const navigate = useUiStore((s) => s.navigate);
  const setCalibOpen = useUiStore((s) => s.setCalibOpen);
  useEffect(() => {
    // Старые ссылки /calibration открывают панель калибровки в упаковке.
    if (route.name === 'calibration') {
      setCalibOpen(true);
      navigate('/pack', { replace: true });
    }
  }, [route.name, navigate, setCalibOpen]);
```
    (import `useEffect` from `react`).

- [ ] **Step 5: Verify**

Run: `bun run typecheck && bun run lint && bun test tests && bun run build`
Expected: pass; `dist/assets` has a separate chunk containing three.js (lazy `BedScene`).

Manual (`bun run dev` + backend):
- choose a 3MF: the scene shows the detected printer's plate (P1S: 18×28 corner light/grey, dimensions «256 мм» on two sides; U1: 4 mm margin band and «отступ 4 мм»), the user's model at the centre with a green projection contour;
- «Уложиться в окно печати» reveals Окно/Запас and «Учитывать реальную смену прутка»; turning the window off hides the AMS option and closes the calibration panel; «Откалибровать…» opens the right panel; `/calibration` opens the same panel;
- start packing: segmented bar fills stage by stage, optional stages show «если понадобится»/«не понадобилась»; during «Пробная нарезка» and «Нарезка стола» the cut plane rises and the cut glows orange;
- success: «Скачать 3MF» primary, «Упаковать заново» secondary, scene rotates by drag;
- light and dark themes both repaint the scene; `prefers-reduced-motion` renders a still frame.

- [ ] **Step 6: Commit**

```bash
git add -A src
git commit -m "Webapp: pack and schedule workspaces, bed scene in packing, calibration as a side panel"
```

---

## Self-review
- Brief v2 §4 (bed per profile, dimensions, no-print zones) → Tasks 1, 4, 7; §5 (AMS option inside the window) → Tasks 6, 9; §6 (real model, projection + contour, slicing) → Tasks 3, 5, 7, 9; calibration from a checkbox into a right panel → Tasks 8, 9; layout «form left, action/progress right» → Tasks 8, 9.
- Placements stay empty in this plan: the scene shows one model at the centre during and after the job. `2026-11-01-packer-layout-event.md` feeds `placements` and `footprintLoops` from the packer.
- DESIGN.md flags green for anything but geometry truth; the schedule page gets no new chart here, so no colour conflict is introduced.
