# Packer `layout` event → backend → bed scene — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Prerequisite:** `2026-11-01-webapp-pack-workspace-scene.md` is merged (`BedScene` with `placements` / `footprintLoops` props).

**Goal:** Show the real packed plate in the webapp scene: after packing (and after every repack and the time-window cut) the packer emits a compact machine-readable `{"event":"layout",…}` with the footprint loops once and `[x, y, angle]` per placement; the backend keeps the latest one for the whole job lifetime (also after success) in both local and server modes; the scene drops the real model's copies onto those positions centre-out and draws the packer's footprint contour and tower.

**Architecture:** Rust: `progress::layout_event` builds the JSON (footprint simplified with RDP from the existing `geo` dependency until the line fits `LAYOUT_EVENT_MAX_BYTES = 16_000`), `progress::layout` prints it when `GOAT_LOG_FORMAT=json`; `colored_3mf_packer` calls it at the top of the verify loop (first pack + every repack, after `order_centre_out`) and after the time-window truncate. Kotlin: `ProgressTracker` remembers the raw line; local mode stores it in `JobSnapshot.layout` → `JobStatusDto.layout`; server mode stores it in a new `queue_jobs.layout_json` column (never truncated, kept after success) → `view()["layout"]`. Both modes now set `GOAT_LOG_FORMAT=json`. Frontend: `PackLayout` type → `BedScene` props.

**Tech Stack:** Rust 2024 (`serde_json`, `geo 0.33`), Kotlin/Spring/JPA + Flyway, React/TS, three.js.

**Facts this plan relies on** (verified in the code on 2026-11-01):
- `PackedPlacement { x_mm, y_mm, angle_deg }` (`src/lib.rs:1401`): `(x, y)` is where the mesh XY bbox centre lands, angle in degrees CCW; footprint loops (`Footprint.loops`, `src/lib.rs:28`) are centred on that same bbox centre, closed, largest first. Drawing = rotate loops by `angle_deg` about (0,0), translate by `(x, y)` — no bbox compensation (that is only for the 3MF transform in `assemble.rs:279`). The webapp's `model3d.ts` centres the mesh the same way.
- `colored_3mf_packer.rs` verify `loop {` (~:669) starts with `order_centre_out(&plan.footprint, &mut packed, centre)` (~:670–672), then `progress::stage(Stage::Assembling, …)` (~:674). Every repack assigns `packed = plan.pack(…)?` and re-enters the loop. The time-window result is `packed.placements.truncate(choice.count)` (~:907–915).
- `PackPlan` has `footprint`, `fixed_keepouts: Vec<Rect>`, `tower_keepout: Option<Rect>`; `usable_bed: Rect` is a local in `run()` (~:578). `Rect { min_x, min_y, max_x, max_y }` (`src/lib.rs:549`).
- Limits on the current path: `BoundedLines.read(limit = 8192)` cuts long lines; `Queue.progress` truncates `stage_json` to 4000 chars; `stage` is hidden once a job leaves RUNNING; local runs never set `GOAT_LOG_FORMAT=json`; `QueueWorker` throttles progress writes to 1/s keyed by stage id+attempt; `JobRunner.kt:260` logs every `"event"` line at INFO.

Line numbers drift — locate by the quoted code, not by number.

---

### Task 1: `progress::layout_event` (Rust, TDD)

**Files:**
- Modify: `src/progress.rs`

- [ ] **Step 1: Failing test** — add to `mod tests` in `src/progress.rs`:

```rust
    #[test]
    fn layout_event_fits_the_line_budget_and_keeps_every_placement() {
        #[derive(serde::Deserialize)]
        struct Loops {
            loops: Vec<Vec<crate::Point>>,
        }
        let fixture: Loops = serde_json::from_str(include_str!("../tests/fixtures/goat4.json")).unwrap();
        let placements: Vec<crate::PackedPlacement> = (0..60)
            .map(|i| crate::PackedPlacement {
                x_mm: 20.0 + (i % 6) as f64 * 37.5,
                y_mm: 20.0 + (i / 6) as f64 * 21.25,
                angle_deg: if i % 2 == 0 { 0.0 } else { 180.0 },
            })
            .collect();
        let bed = crate::Rect { min_x: 0.0, min_y: 0.0, max_x: 256.0, max_y: 256.0 };
        let corner = crate::Rect { min_x: 0.0, min_y: 0.0, max_x: 22.0, max_y: 32.0 };
        let event = layout_event(LayoutSource::Pack, &fixture.loops, &placements, bed, &[corner], None);

        assert!(event.to_string().len() <= LAYOUT_EVENT_MAX_BYTES, "{} bytes", event.to_string().len());
        assert_eq!(event["event"], "layout");
        assert_eq!(event["source"], "pack");
        assert_eq!(event["objects"], 60);
        assert_eq!(event["placements"].as_array().unwrap().len(), 60);
        assert_eq!(event["placements"][1], serde_json::json!([57.5, 20.0, 180.0]));
        assert_eq!(event["bed"], serde_json::json!([0.0, 0.0, 256.0, 256.0]));
        assert!(event["tower"].is_null());
        let ring = event["footprint"][0].as_array().unwrap();
        assert!(ring.len() >= 4 && ring.len() < fixture.loops[0].len(), "simplified: {} points", ring.len());
        assert_eq!(ring.first(), ring.last(), "rings stay closed");
    }

    #[test]
    fn layout_source_ids() {
        assert_eq!(LayoutSource::Pack.id(), "pack");
        assert_eq!(LayoutSource::Repack.id(), "repack");
        assert_eq!(LayoutSource::Window.id(), "window");
    }
```

If `goat4.json`'s top-level key is not `loops`, open the file (`head -c 200 tests/fixtures/goat4.json`) and match the struct to it; if `Rect` fields are not `pub`, build the rects with the constructor `lib.rs` provides.

- [ ] **Step 2: Run** — `cargo test --release layout_` → FAIL (`layout_event` not found).

- [ ] **Step 3: Implement** — add to `src/progress.rs` (above `mod tests`):

```rust
use geo::{Coord, LineString, Simplify};

/// Upper bound for one `layout` line. The webapp polls job status every second and the backend
/// reads packer output with a 64 KiB line cap, so the footprint is simplified until it fits.
pub const LAYOUT_EVENT_MAX_BYTES: usize = 16_000;

/// Which pass produced the layout the webapp should draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutSource {
    /// First winning layout.
    Pack,
    /// Layout after a repack (grown tower, support contact/strict, corner tower failed).
    Repack,
    /// Prefix kept for the print-time window.
    Window,
}

impl LayoutSource {
    pub fn id(self) -> &'static str {
        match self {
            LayoutSource::Pack => "pack",
            LayoutSource::Repack => "repack",
            LayoutSource::Window => "window",
        }
    }
}

fn round_to(value: f64, step: f64) -> f64 {
    (value / step).round() * step
}

fn rect_json(rect: &crate::Rect) -> serde_json::Value {
    json!([round_to(rect.min_x, 0.1), round_to(rect.min_y, 0.1), round_to(rect.max_x, 0.1), round_to(rect.max_y, 0.1)])
}

/// Footprint rings simplified with Ramer–Douglas–Peucker at `epsilon_mm`, rounded to 0.01 mm.
/// Rings that collapse below a triangle are dropped.
fn simplified_loops(loops: &[Vec<crate::Point>], epsilon_mm: f64) -> Vec<Vec<[f64; 2]>> {
    loops
        .iter()
        .filter_map(|ring| {
            let line: LineString<f64> = ring.iter().map(|p| Coord { x: p.x, y: p.y }).collect();
            let simple = line.simplify(&epsilon_mm);
            let points: Vec<[f64; 2]> = simple.0.iter().map(|c| [round_to(c.x, 0.01), round_to(c.y, 0.01)]).collect();
            (points.len() >= 4).then_some(points)
        })
        .collect()
}

/// The `layout` event: footprint loops once (centred on the mesh bbox centre, mm) plus
/// `[x_mm, y_mm, angle_deg]` per placement — the webapp rotates the loops by the angle and
/// translates them, the same math as `placement_world_loops`.
pub fn layout_event(
    source: LayoutSource,
    loops: &[Vec<crate::Point>],
    placements: &[crate::PackedPlacement],
    bed: crate::Rect,
    keepouts: &[crate::Rect],
    tower: Option<crate::Rect>,
) -> serde_json::Value {
    let placements_json: Vec<serde_json::Value> = placements
        .iter()
        .map(|p| json!([round_to(p.x_mm, 0.01), round_to(p.y_mm, 0.01), round_to(p.angle_deg, 0.1)]))
        .collect();
    let mut epsilon_mm = 0.1;
    loop {
        let event = json!({
            "event": "layout",
            "source": source.id(),
            "objects": placements.len(),
            "bed": rect_json(&bed),
            "keepouts": keepouts.iter().map(rect_json).collect::<Vec<_>>(),
            "tower": tower.as_ref().map(rect_json),
            "footprint": simplified_loops(loops, epsilon_mm),
            "placements": placements_json,
            "epsilon_mm": epsilon_mm,
        });
        // A coarse contour is still a faithful picture of where things stand; give up shrinking
        // past 5 mm (hundreds of tiny objects) and let the backend's 64 KiB line cap decide.
        if event.to_string().len() <= LAYOUT_EVENT_MAX_BYTES || epsilon_mm >= 5.0 {
            return event;
        }
        epsilon_mm *= 2.0;
    }
}

/// Prints the layout event for the webapp (machine-readable mode only; the human log already
/// says how many objects were packed).
pub fn layout(
    source: LayoutSource,
    loops: &[Vec<crate::Point>],
    placements: &[crate::PackedPlacement],
    bed: crate::Rect,
    keepouts: &[crate::Rect],
    tower: Option<crate::Rect>,
) {
    if structured() {
        eprintln!("{}", layout_event(source, loops, placements, bed, keepouts, tower));
    }
}
```

If `line.simplify(&epsilon_mm)` does not compile because this `geo` version takes the epsilon by value, call `line.simplify(epsilon_mm)`.

- [ ] **Step 4: Run** — `cargo test --release layout_` → PASS (2 tests). Then `cargo test --release progress` → all progress tests PASS.

- [ ] **Step 5: Commit**

```bash
git add src/progress.rs
git commit -m "progress: layout event (simplified footprint + placements) for the webapp scene"
```

---

### Task 2: Emit the layout from the packer

**Files:**
- Modify: `src/bin/colored_3mf_packer.rs`

- [ ] **Step 1: Before the verify loop** (the line `loop {` that is followed by `order_centre_out(&plan.footprint, &mut packed, centre)`), add:

```rust
    // The webapp scene draws every winning layout; the first one is "pack", later ones "repack".
    let mut layout_source = progress::LayoutSource::Pack;
```

- [ ] **Step 2: Inside the loop, right after `order_centre_out(…)` and before `progress::stage(Stage::Assembling, …)`**, add:

```rust
        progress::layout(
            layout_source,
            &plan.footprint.loops,
            &packed.placements,
            usable_bed,
            &plan.fixed_keepouts,
            plan.tower_keepout,
        );
        layout_source = progress::LayoutSource::Repack;
```

(`Rect` is `Copy` if `usable_bed`/`tower_keepout` are used by value elsewhere in `run()`; otherwise pass `usable_bed.clone()` and `plan.tower_keepout.clone()`.)

- [ ] **Step 3: After the time-window cut** — right after `packed.placements.truncate(choice.count);` add:

```rust
            progress::layout(
                progress::LayoutSource::Window,
                &plan.footprint.loops,
                &packed.placements,
                usable_bed,
                &plan.fixed_keepouts,
                plan.tower_keepout,
            );
```

- [ ] **Step 4: Verify** — `cargo build --release && cargo test --release --all` → PASS. Spot-check the event on a fixture without a slicer is not possible (the binary slices); instead confirm the call sites compile and run the unit test from Task 1. The end-to-end check is Task 6.

- [ ] **Step 5: Commit**

```bash
git add src/bin/colored_3mf_packer.rs
git commit -m "colored_3mf_packer: emit layout after each pack/repack and after the time-window cut"
```

---

### Task 3: ProgressTracker keeps the latest layout (Kotlin, TDD)

**Files:**
- Modify: `webapp/backend/src/main/kotlin/com/goatpacker/web/service/ProgressTracker.kt`
- Test: `webapp/backend/src/test/kotlin/com/goatpacker/web/service/PackStageProgressTest.kt`

- [ ] **Step 1: Failing test** — add to `PackStageProgressTest` (it already has `FakeClock`, `tracker(clock)` and the `stage(id, expected, objects)` JSON-line helper):

```kotlin
    @Test
    fun `layout-событие запоминается как есть и не трогает этап и сообщение`() {
        val t = tracker(FakeClock())
        val packing = t.onLine(stage("packing", 20.0, null))
        val layout = """{"event":"layout","source":"pack","objects":2,"bed":[0,0,256,256],"keepouts":[],"tower":null,"footprint":[[[0,0],[1,0],[1,1],[0,0]]],"placements":[[10,10,0],[40,10,180]]}"""

        val event = t.onLine(layout)

        assertEquals(layout, event.layout)
        assertEquals(packing.stage?.id, event.stage?.id)
        assertEquals(packing.phaseMessage, event.phaseMessage)
        assertEquals(layout, t.onLine(stage("assembling", null, 2)).layout, "раскладка живёт до следующей")
    }
```

Run (from `webapp/backend`): `./gradlew test --tests '*PackStageProgressTest*'` → FAIL (`layout` unresolved).

- [ ] **Step 2: Implement.**
  - `data class ProgressEvent(…)`: add the last parameter

    ```kotlin
        /** Последнее layout-событие пакера (сырой JSON) — для сцены стола; null — раскладки ещё нет. */
        val layout: String? = null,
    ```
  - In `class ProgressTracker` add `private var layoutJson: String? = null`.
  - In `onLine`, in the PACK branch, before `if (event != null) onPackEvent(event) …`, add:

    ```kotlin
            if (event != null && event.path("event").asText() == "layout") {
                layoutJson = line.trim()
                return snapshot()
            }
    ```
  - In `snapshot()` pass `layout = layoutJson` to the `ProgressEvent(…)` constructor.

- [ ] **Step 3: Run** — `./gradlew test --tests '*PackStageProgressTest*'` → PASS; `./gradlew test` → all PASS.

- [ ] **Step 4: Commit**

```bash
git add webapp/backend/src
git commit -m "Backend: progress tracker keeps the packer's latest layout event"
```

---

### Task 4: Local mode — snapshot, DTO, JSON events on, quiet log, longer lines

**Files:**
- Modify: `webapp/backend/src/main/kotlin/com/goatpacker/web/job/Job.kt` (`JobSnapshot`)
- Modify: `webapp/backend/src/main/kotlin/com/goatpacker/web/job/JobRunner.kt`
- Modify: `webapp/backend/src/main/kotlin/com/goatpacker/web/job/BoundedLines.kt`
- Modify: `webapp/backend/src/main/kotlin/com/goatpacker/web/dto/JobDtos.kt` (`JobStatusDto`)
- Modify: `webapp/backend/src/main/kotlin/com/goatpacker/web/service/JobService.kt`
- Modify: `webapp/backend/src/main/kotlin/com/goatpacker/web/server/NativeLauncher.kt` (local runner branch)
- Test: the existing local pack-job integration test + `webapp/backend/src/test/resources/bin/fake-packer.sh`

- [ ] **Step 1: Failing test.** Find the local-mode test that runs a pack job through `fake-packer.sh` to SUCCEEDED: `grep -rln "fake-packer" webapp/backend/src/test`. In `fake-packer.sh`, next to where it prints its other output, add:

```sh
if [ "$GOAT_LOG_FORMAT" = "json" ]; then
  echo '{"event":"layout","source":"pack","objects":2,"bed":[0,0,256,256],"keepouts":[],"tower":null,"footprint":[[[-5,-5],[5,-5],[5,5],[-5,5],[-5,-5]]],"placements":[[20,20,0],[40,20,90]]}' >&2
fi
```

In that test, after the job reaches `SUCCEEDED`, assert on the status JSON (adapt the accessor to the test's style — it reads `/api/jobs/{id}` as a `JsonNode` or DTO):

```kotlin
        assertEquals(2, status["layout"]["placements"].size(), "раскладка видна и после успеха")
        assertEquals("pack", status["layout"]["source"].asText())
```

Run `./gradlew test --tests '<that test class>'` → FAIL (`layout` missing).

- [ ] **Step 2: Implement.**
  - `JobSnapshot`: add `val layout: String? = null,` (last field).
  - `JobRunner`: in the plain `ProcessBuilder(command)` chain (local, non-launcher path) add before `.start()`:

    ```kotlin
                    .also { it.environment()["GOAT_LOG_FORMAT"] = "json" }
    ```
    In the `job.update { it.copy(…) }` inside the line loop add `layout = event.layout ?: it.layout,`.
    Replace the log line

    ```kotlin
                    if (line.contains("event=") || line.contains("\"event\"")) log.info("jobId={} component=cli message={}", job.id, line.take(8192))
    ```
    with

    ```kotlin
                    // layout — до 16 КБ на строку: в лог только факт, иначе он забивается контурами.
                    if (line.contains("\"event\":\"layout\"")) log.info("jobId={} component=cli event=layout bytes={}", job.id, line.length)
                    else if (line.contains("event=") || line.contains("\"event\"")) log.info("jobId={} component=cli message={}", job.id, line.take(8192))
    ```
  - `NativeLauncher.start`, local branch: extend the `.also { … }` so it also sets `it.environment()["GOAT_LOG_FORMAT"]="json"` (the docker branch already passes `--env GOAT_LOG_FORMAT=json`).
  - `BoundedLines.read`: change the default `limit` from `8192` to `65536` (layout lines are ≤ 16 000 bytes by construction; the cap still protects memory).
  - `JobStatusDto`: add `val layout: com.fasterxml.jackson.databind.JsonNode? = null,` (last field; the class is `@JsonInclude(ALWAYS)`).
  - `JobService`: add `private val layoutMapper = com.fasterxml.jackson.databind.ObjectMapper()` to the class and, in the `JobStatusDto(…)` construction, `layout = snap.layout?.let { runCatching { layoutMapper.readTree(it) }.getOrNull() },` — not gated on RUNNING: the done scene needs it.

- [ ] **Step 3: Run** — the test from Step 1 → PASS; `./gradlew test` → all PASS. If an existing local-mode test asserted text-only stage detection with `GOAT_LOG_FORMAT` unset, it keeps passing because `fake-packer.sh` prints its text lines regardless; if a test now sees JSON lines it did not expect, update its expectation to the structured stage (the tracker already handles both).

- [ ] **Step 4: Commit**

```bash
git add webapp/backend/src
git commit -m "Backend (local): JSON progress events on; layout in job status, kept after success"
```

---

### Task 5: Server mode — `layout_json` column

**Files:**
- Create: `webapp/backend/src/main/resources/db/migration/V4__job_layout.sql`
- Modify: `webapp/backend/src/main/kotlin/com/goatpacker/web/server/Model.kt`, `Queue.kt`, `QueueWorker.kt`
- Test: `webapp/backend/src/test/kotlin/com/goatpacker/web/server/ServerIntegrationTest.kt`

- [ ] **Step 1: Failing test.** In `ServerIntegrationTest`, in the test that runs a PACK job to `SUCCEEDED` through the local runner (search `"PACK"` / `SUCCEEDED` in the file), assert on the `/api/v2/jobs/{id}` body:

```kotlin
        assertEquals(2, body["layout"]["placements"].size())
```

Run `./gradlew test --tests '*ServerIntegrationTest*'` → FAIL.

- [ ] **Step 2: Migration** `V4__job_layout.sql`:

```sql
-- Append-only migration: последняя раскладка пакера (JSON layout-события) для сцены стола.
-- Отдельно от stage_json: живёт и после успеха, не обрезается.
alter table queue_jobs add column layout_json text;
```

- [ ] **Step 3: Entity** — in `QueueJobEntity` (`Model.kt`) after `stageJson`:

```kotlin
    // Последняя раскладка пакера (JSON layout-события); null — раскладки нет (V4).
    @Column(columnDefinition="text") var layoutJson: String? = null,
```

- [ ] **Step 4: Queue.**
  - `progress(...)`: add the parameter `layoutJson:String?=null` (last) and in the body `if(layoutJson!=null) j.layoutJson=layoutJson` (no truncation; null keeps the previous layout).
  - The retry path that sets `j.stageJson=null` also sets `j.layoutJson=null`.
  - `view(...)`: add to the returned map `"layout" to j.layoutJson?.let { runCatching { mapper.readTree(it) }.getOrNull() }` (not gated on status).

- [ ] **Step 5: Worker** — in `QueueWorker`, extend the throttled `job.onUpdate` so a new layout is written immediately and only once:

```kotlin
            var lastLayout:String?=null
            job.onUpdate={snap -> val stageKey=snap.stage?.let { it.id to it.attempt }
                val newLayout=snap.layout?.takeIf { it!=lastLayout }
                if(System.nanoTime()-last>TimeUnit.SECONDS.toNanos(1) || snap.status!=JobStatus.RUNNING || stageKey!=lastStage || newLayout!=null) {
                last=System.nanoTime();lastStage=stageKey;if(newLayout!=null) lastLayout=newLayout
                queue.progress(record.id,record.fence,snap.phase.jsonValue(),snap.progressPercent,snap.phaseMessage,snap.stage?.let(mapper::writeValueAsString),newLayout)
            }}
```

- [ ] **Step 6: Run** — `./gradlew test` → all PASS (Flyway applies V4 in the test database).

- [ ] **Step 7: Commit**

```bash
git add webapp/backend/src
git commit -m "Backend (server): layout_json column, layout in /api/v2/jobs, written on change"
```

---

### Task 6: Frontend — type, conversion (TDD), scene wiring with drop animation

**Files:**
- Modify: `webapp/frontend/src/api/types.ts`
- Create: `webapp/frontend/src/scene/layout.ts`, test `webapp/frontend/tests/layout.test.ts`
- Modify: `webapp/frontend/src/scene/BedScene.tsx`, `webapp/frontend/src/pages/PackPage.tsx`

- [ ] **Step 1: Type** — in `types.ts`:

```ts
/** layout-событие пакера: контур (мм, вокруг центра габарита модели) и позиции копий. */
export interface PackLayout {
  source: 'pack' | 'repack' | 'window';
  objects: number;
  /** usable bed [x0, y0, x1, y1], мм. */
  bed: number[];
  keepouts: number[][];
  tower: number[] | null;
  footprint: [number, number][][];
  /** [x_mm, y_mm, angle_deg] — центр габарита модели и поворот против часовой. */
  placements: [number, number, number][];
}
```
and in `JobStatusResponse` add `layout?: PackLayout | null;`.

- [ ] **Step 2: Failing test `tests/layout.test.ts`**

```ts
import { describe, expect, test } from 'bun:test';
import type { PackLayout } from '../src/api/types';
import { sceneLayout } from '../src/scene/layout';

const layout: PackLayout = {
  source: 'repack', objects: 2, bed: [0, 0, 256, 256], keepouts: [[0, 0, 22, 32]], tower: [214, 214, 246, 246],
  footprint: [[[-5, -5], [5, -5], [5, 5], [-5, -5]]], placements: [[20, 20, 0], [40, 20, 90]],
};

describe('sceneLayout', () => {
  test('placements become scene placements in packer order (centre-out)', () => {
    expect(sceneLayout(layout).placements).toEqual([
      { x: 20, y: 20, angleDeg: 0 },
      { x: 40, y: 20, angleDeg: 90 },
    ]);
  });
  test('tower becomes a rect; missing layout gives an empty scene layout', () => {
    expect(sceneLayout(layout).tower).toEqual({ x0: 214, y0: 214, x1: 246, y1: 246 });
    expect(sceneLayout(null)).toEqual({ placements: [], footprintLoops: null, tower: null });
  });
});
```

Run `bun test tests/layout.test.ts` → FAIL.

- [ ] **Step 3: Implement `src/scene/layout.ts`**

```ts
import type { PackLayout } from '../api/types';
import type { Rect } from './bed';
import type { Placement } from './BedScene';

export interface SceneLayout {
  placements: Placement[];
  footprintLoops: [number, number][][] | null;
  tower: Rect | null;
}

/** layout-событие пакера → пропсы сцены. Порядок копий — порядок пакера (от центра). */
export function sceneLayout(layout: PackLayout | null | undefined): SceneLayout {
  if (!layout) return { placements: [], footprintLoops: null, tower: null };
  const [x0, y0, x1, y1] = layout.tower ?? [];
  return {
    placements: layout.placements.map(([x, y, angleDeg]) => ({ x, y, angleDeg })),
    footprintLoops: layout.footprint.length ? layout.footprint : null,
    tower: layout.tower ? { x0, y0, x1, y1 } : null,
  };
}
```

Because `layout.ts` imports the `Placement` type from `BedScene.tsx`, move `export interface Placement` from `BedScene.tsx` into `layout.ts` and import it back in `BedScene.tsx` (`import type { Placement } from './layout'`) — `bun test` must not load three.js.

Run `bun test tests/layout.test.ts` → PASS.

- [ ] **Step 4: BedScene — tower and drop animation.** In `BedScene.tsx`:
  - add the prop `tower?: Rect | null;` (import `type Rect` from `./bed`);
  - the tower belongs to the job, not to the printer, so draw it in `buildObjects` (not `buildBed`), after adding `footprints`:

    ```tsx
      const tower = propsRef.current.tower;
      if (tower) {
        towerMesh = new THREE.Mesh(new THREE.ShapeGeometry(shapeOf(rectPoints(tower))), new THREE.MeshBasicMaterial({ transparent: true, opacity: 0.35, depthWrite: false, color: cssColor('--accent-2') }));
        towerMesh.position.z = 0.04;
        scene.add(towerMesh);
      }
    ```
    with `let towerMesh: THREE.Mesh | null = null;` next to `solid`, and remove/dispose it at the top of `buildObjects` like the other meshes; include `p.tower` in the rebuild key (`JSON.stringify([p.placements ?? [], p.tower ?? null])`) and in the effect dependency list.
  - drop animation: keep the computed `spots` and a start time in closure variables (`let spots: Placement[] = []; let dropStart = 0;`), set them in `buildObjects` (`dropStart = placements.length ? performance.now() : 0`). In `tick`, before `renderer.render`, when `solid && dropStart && performance.now() - dropStart < 1400`:

    ```tsx
        const t = (performance.now() - dropStart) / 1400;
        const m = new THREE.Matrix4(), q = new THREE.Quaternion(), s = new THREE.Vector3(1, 1, 1), z = new THREE.Vector3(0, 0, 1);
        spots.forEach((p, i) => {
          // По порядку пакера — от центра наружу; каждая копия падает за ~40 % общего времени.
          const local = Math.min(1, Math.max(0, (t - (i / spots.length) * 0.6) / 0.4));
          const eased = 1 - (1 - local) ** 3;
          q.setFromAxisAngle(z, (p.angleDeg * Math.PI) / 180);
          m.compose(new THREE.Vector3(p.x, p.y, local > 0 ? (1 - eased) * 120 : -9999), q, s);
          solid!.setMatrixAt(i, m);
          cap!.setMatrixAt(i, m);
        });
        solid.instanceMatrix.needsUpdate = true;
        cap!.instanceMatrix.needsUpdate = true;
    ```
    After the window ends, set the final matrices once (call the same loop with `local = 1`) and `dropStart = 0`. Under `prefers-reduced-motion` skip the animation (`dropStart = 0` in `buildObjects`).

- [ ] **Step 5: PackPage wiring** — compute and pass the layout:

```tsx
  const placed = sceneLayout(job?.layout);
```
```tsx
            <BedScene
              …existing props…
              placements={placed.placements}
              footprintLoops={placed.footprintLoops}
              tower={placed.tower}
              readout={placed.placements.length ? `${placed.placements.length} объектов на столе` : model ? `${model.size.map((v) => Math.round(v)).join(' × ')} мм` : ''}
            />
```
(import `sceneLayout` from `@/scene/layout`). Add «башня очистки» to the scene legend when `tower` is set (`<span>башня очистки <i className="swatch swatch-tower" /></span>`, CSS `.swatch-tower { background: var(--accent-2-soft); border: 1px dashed var(--accent-2); }`).

- [ ] **Step 6: Verify**

Run: `bun run typecheck && bun run lint && bun test tests && bun run build` → pass.

End-to-end (real packer + slicer on a dev machine, local mode): `cargo build --release`, backend `./gradlew bootRun`, frontend `bun run dev`; pack a real 3MF:
- after «Раскладка» the copies of the real model drop onto the plate centre-out and the green contour under each matches the packer's footprint (with supports, the contour is wider than the mesh);
- the tower overlay sits where the slicer will put it; on a repack the plate rebuilds;
- with «Уложиться в окно печати», the plate shrinks to the kept prefix («window»);
- after success the full plate stays and rotates; reload the page → `layout` is still in `/api/jobs/{id}` and the scene shows it.
In server mode repeat once and check `select length(layout_json) from queue_jobs order by created_at desc limit 1;` is under 16 000.

- [ ] **Step 7: Commit**

```bash
git add webapp/frontend
git commit -m "Webapp: real packed layout in the bed scene (copies drop centre-out, packer contour, tower)"
```

---

## Self-review
- Every winning layout reaches the scene: first pack + every repack (Task 2 Step 2), time-window prefix (Step 3).
- Size limits: event ≤ 16 000 bytes by construction (Task 1), backend line cap raised to 64 KiB (Task 4), server column untruncated and separate from `stage_json` (Task 5); logs no longer carry contours (Task 4).
- Lifetime: local snapshot and server column survive success; only a retry clears the server copy (Task 5).
- Truthfulness: positions and contour come from the packer; the mesh is the user's own file; the drop animation is a replay of a finished layout, not a simulated search (DESIGN.md: no fake progress when real stage data exists).
- `CLAUDE.md` updates once implemented: under `colored_3mf_packer` invariants add «Progress: `progress::layout` prints a `{"event":"layout"}` (footprint RDP-simplified to ≤ 16 KB + placements) after each pack/repack and the time-window cut; the webapp keeps the latest one per job (`layout_json`)».
