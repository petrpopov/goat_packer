---
name: Goat Packer
description: Stone & Spark, a warm-stone operating UI with one signal-orange spark for actions and machine progress.
colors:
  # Light ("Камень", stone) is the base key; the dark (warm charcoal) value of the
  # same role is the sibling key with the "-dark" suffix.
  bg: "#ebe7e2"
  bg-dark: "#121110"
  chrome: "#e3ded8"
  chrome-dark: "#0f0e0d"
  panel: "#f8f6f3"
  panel-dark: "#1b1918"
  panel-2: "#e0dbd5"
  panel-2-dark: "#23201e"
  hover: "#d9d3cc"
  hover-dark: "#2a2724"
  border: "#cdc6be"
  border-dark: "#34302c"
  border-strong: "#aba299"
  border-strong-dark: "#45403a"
  text: "#1b1916"
  text-dark: "#ece8e3"
  muted: "#5a5249"
  muted-dark: "#a49d95"
  dim: "#5a5249"
  dim-dark: "#958e86"
  accent: "#9c3d00"
  accent-dark: "#ff9640"
  accent-fill: "#ff7a1a"
  accent-fill-dark: "#ff9640"
  accent-ink: "#1a0d00"
  accent-ink-dark: "#1a0d00"
  accent-soft: "#ff7a1a26"
  accent-soft-dark: "#ff964024"
  signal: "#ff7a1a"
  signal-dark: "#ff9640"
  accent-2: "#006b4a"
  accent-2-dark: "#3ddc91"
  accent-2-soft: "#006b4a1f"
  accent-2-soft-dark: "#3ddc9124"
  ok: "#1f7a3a"
  ok-dark: "#3ddc91"
  ok-soft: "#1f7a3a1f"
  ok-soft-dark: "#3ddc911f"
  warn: "#9a5b00"
  warn-dark: "#ffcd48"
  warn-soft: "#9a5b001f"
  warn-soft-dark: "#ffcd4824"
  crit: "#c42847"
  crit-dark: "#ff6285"
  crit-soft: "#c428471a"
  crit-soft-dark: "#ff628524"
  scene-bg: "#e4dfd9"
  scene-bg-dark: "#161413"
  scene-plate: "#2b2724"
  scene-plate-dark: "#070606"
  scene-plate-top: "#faf8f5"
  scene-plate-top-dark: "#221f1c"
  scene-plate-grid: "#e2dcd4"
  scene-plate-grid-dark: "#302c28"
  scene-keepout: "#8f877e"
  scene-keepout-dark: "#ece8e3"
  scene-object: "#aea69c"
  scene-object-dark: "#c2bbb3"
typography:
  page-title:
    fontFamily: "IBM Plex Sans, -apple-system, system-ui, sans-serif"
    fontSize: "20px"
    fontWeight: 600
    lineHeight: 1.3
    letterSpacing: "-0.015em"
  title:
    fontFamily: "IBM Plex Sans, -apple-system, system-ui, sans-serif"
    fontSize: "18px"
    fontWeight: 600
    lineHeight: 1.3
    letterSpacing: "-0.015em"
  section:
    fontFamily: "IBM Plex Sans, -apple-system, system-ui, sans-serif"
    fontSize: "15px"
    fontWeight: 600
    lineHeight: 1.35
  body:
    fontFamily: "IBM Plex Sans, -apple-system, system-ui, sans-serif"
    fontSize: "13px"
    fontWeight: 400
    lineHeight: 1.45
    letterSpacing: "-0.005em"
  label:
    fontFamily: "IBM Plex Sans, -apple-system, system-ui, sans-serif"
    fontSize: "13px"
    fontWeight: 500
    lineHeight: 1.45
  hint:
    fontFamily: "IBM Plex Sans, -apple-system, system-ui, sans-serif"
    fontSize: "12px"
    fontWeight: 400
    lineHeight: 1.4
  data:
    fontFamily: "JetBrains Mono, ui-monospace, Menlo, monospace"
    fontSize: "13px"
    fontWeight: 400
    lineHeight: 1.45
    fontFeature: "\"tnum\" 1"
  data-stat:
    fontFamily: "JetBrains Mono, ui-monospace, Menlo, monospace"
    fontSize: "15px"
    fontWeight: 500
    lineHeight: 1.3
    fontFeature: "\"tnum\" 1"
rounded:
  sm: "4px"
  md: "6px"
  lg: "10px"
  pill: "999px"
spacing:
  "2xs": "4px"
  xs: "6px"
  sm: "10px"
  md: "12px"
  lg: "16px"
  xl: "18px"
  "2xl": "24px"
components:
  button-primary:
    backgroundColor: "{colors.accent-fill}"
    textColor: "{colors.accent-ink}"
    typography: "{typography.label}"
    rounded: "{rounded.md}"
    padding: "0 14px"
    height: "32px"
  button-primary-lg:
    backgroundColor: "{colors.accent-fill}"
    textColor: "{colors.accent-ink}"
    rounded: "{rounded.md}"
    padding: "0 20px"
    height: "40px"
  button-secondary:
    backgroundColor: "{colors.panel}"
    textColor: "{colors.text}"
    typography: "{typography.label}"
    rounded: "{rounded.md}"
    padding: "0 14px"
    height: "32px"
  button-secondary-hover:
    backgroundColor: "{colors.hover}"
  button-icon:
    textColor: "{colors.muted}"
    rounded: "{rounded.md}"
    size: "32px"
  input:
    backgroundColor: "{colors.panel}"
    textColor: "{colors.text}"
    typography: "{typography.body}"
    rounded: "{rounded.md}"
    padding: "0 10px"
    height: "32px"
  segmented:
    backgroundColor: "{colors.panel-2}"
    rounded: "{rounded.md}"
    padding: "2px"
  segmented-option-on:
    backgroundColor: "{colors.panel}"
    textColor: "{colors.text}"
    rounded: "{rounded.sm}"
    height: "26px"
  nav-item:
    textColor: "{colors.muted}"
    typography: "{typography.label}"
    rounded: "{rounded.md}"
    padding: "0 11px"
    height: "34px"
  nav-item-active:
    backgroundColor: "{colors.accent-soft}"
    textColor: "{colors.text}"
  topbar:
    backgroundColor: "{colors.chrome}"
    height: "52px"
  sidebar:
    backgroundColor: "{colors.chrome}"
    width: "208px"
  sidebar-rail:
    backgroundColor: "{colors.chrome}"
    width: "56px"
  file-chip:
    backgroundColor: "{colors.panel}"
    rounded: "{rounded.lg}"
    padding: "10px 10px 10px 12px"
  progress-bar:
    backgroundColor: "{colors.panel-2}"
    rounded: "{rounded.sm}"
    height: "8px"
  result-banner-ok:
    backgroundColor: "{colors.ok-soft}"
    rounded: "{rounded.lg}"
    padding: "14px 16px"
  result-banner-err:
    backgroundColor: "{colors.crit-soft}"
    rounded: "{rounded.lg}"
    padding: "14px 16px"
  pill-ok:
    backgroundColor: "{colors.ok-soft}"
    textColor: "{colors.ok}"
    typography: "{typography.hint}"
    rounded: "{rounded.pill}"
    padding: "2px 9px"
  pill-running:
    backgroundColor: "{colors.accent-soft}"
    textColor: "{colors.accent}"
    rounded: "{rounded.pill}"
    padding: "2px 9px"
  pill-error:
    backgroundColor: "{colors.crit-soft}"
    textColor: "{colors.crit}"
    rounded: "{rounded.pill}"
    padding: "2px 9px"
  pill-queued:
    backgroundColor: "{colors.panel-2}"
    textColor: "{colors.muted}"
    rounded: "{rounded.pill}"
    padding: "2px 9px"
  table-row:
    backgroundColor: "{colors.panel}"
    padding: "0 14px"
    height: "44px"
  menu:
    backgroundColor: "{colors.panel}"
    rounded: "{rounded.lg}"
    padding: "6px"
    width: "230px"
  auth-card:
    width: "380px"
  scene-card:
    backgroundColor: "{colors.scene-bg}"
    rounded: "{rounded.lg}"
---

# Design System: Goat Packer

## Overview

**Creative North Star: "Камень и искра" (Stone & Spark)**

The interface is a workshop bench: warm stone in daylight, warm charcoal at night, dense and quiet so the work on it is what you see. One bright signal orange is the spark. It marks what the user can do (the primary action) and what the machine is doing right now (progress, the slicing plane). A single green is reserved for geometry truth: the measured projection contour, the tower, "готово". Everything else is tonal stone.

Mode is **Operate**: a product UI used for minutes at a time next to a slicer. Precision and restraint beat expression. Familiar patterns, 13px body, tight but legible density, numbers in a monospaced face that never jitters. Brand lives in the details (the spark, the scene), not in decoration.

It is warm but deliberately not the Anthropic look: dense grey-beige stone instead of pale cream, a loud signal orange instead of clay, a grotesque instead of a serif.

**Key Characteristics:**
- Two themes of one material: stone (light) and warm charcoal (dark), switched with `data-theme`.
- One spark (orange) for action and machine progress; one green for geometry truth and success.
- Flat tonal layering; shadows only on floating layers.
- IBM Plex Sans for UI, JetBrains Mono with tabular figures for every number.
- Thin 24px stroke icons from one sprite.

## Colors

Warm, low-chroma neutrals carry the whole surface; chroma is spent only on the spark and on truth.

**Token convention.** Each frontmatter key is the light ("Камень") value; the same role in the dark theme is the sibling key with a `-dark` suffix (`accent` / `accent-dark`). In CSS both map to one custom property (`--accent`) redefined under `:root[data-theme="dark"]`. Components reference the base key; the theme picks the value.

### Primary
- **Spark Orange, text** (`accent` / `accent-dark`): links, active nav icon, running pill, ID links in tables. On light it is the burnt `#9c3d00` (5.5:1 on bg); on dark the bright orange (8.7:1 on bg).
- **Spark Orange, fill** (`accent-fill`): the primary button and Gantt print bars. Always with `accent-ink` text (7.3:1 on light fill, 8.8:1 on dark fill).
- **Signal** (`signal`): machine progress only: done/current progress segments, current stage dot, the rising slice plane in the scene.
- **Spark Haze** (`accent-soft`): active nav background, input focus ring, selection, calculation readouts.

### Secondary
- **Geometry Green** (`accent-2`, `accent-2-soft`): projection contour, tower keepout, "готово" moments. On light `#006b4a` (5.3:1 on bg).

### Status
- **ok / warn / crit** with `-soft` tints for banners and pills. `warn` also marks stubs ("скоро", "заготовка").

### Neutral
- **Bench Stone / Charcoal** (`bg`): page background.
- **Chrome** (`chrome`): topbar and sidebar, one step darker than bg.
- **Panel / Panel-2** (`panel`, `panel-2`): raised working surfaces; panel-2 is the recessed well (segmented track, table header, progress track, notes).
- **Hover** (`hover`): row and ghost-button hover.
- **Border / Border-strong**: hairlines between regions; strong for control outlines.
- **Text / Muted / Dim**: primary copy (14.3:1 on light bg), secondary copy, hints. On light, muted and dim share `#5a5249` (6.2:1) so hints never drop below AA.

### Scene (Goat Packer-specific)
- `scene-bg`, `scene-plate` (the dark build-plate edge), `scene-plate-top` (usable area), `scene-plate-grid`, `scene-keepout` (forbidden zones), `scene-object` (placed silhouettes, flat fill). The projection contour and tower use `accent-2`; the slicing plane uses `signal`.

### Named Rules
**The One Spark Rule.** Orange appears only where the user acts (`accent-fill`) or the machine works (`signal`). Never as decoration, section color or illustration fill.

**The Burnt-on-Stone Rule.** On light surfaces orange text is only `#9c3d00`. `#ff7a1a` is fill-only on light (2.4:1 as text, fails).

**The Truth Is Green Rule.** Green means "measured and correct": projection contour, tower, success. Not for categories, not for charts, not for night bands.

## Typography

**Body Font:** IBM Plex Sans (with -apple-system, system-ui, sans-serif)
**Label/Mono Font:** JetBrains Mono (with ui-monospace, Menlo, monospace)

**Character:** A sober grotesque with a technical edge, paired with a mono that makes numbers read like instrument readouts. Weights 400 · 500 · 600 only.

**Licensing:** both are SIL OFL 1.1. Self-host via `@fontsource/ibm-plex-sans` and `@fontsource/jetbrains-mono` (weights 400/500/600 and 400/500, Cyrillic + Latin subsets); ship the OFL license text with the build. No Google Fonts CDN in production.

### Hierarchy
- **Page title** (600, 20px, -0.015em): page headings (Мои задачи, задача #id). Auth card heading is 22px, -0.02em.
- **Title** (600, 18px): the action-bar file name on the work screen, one line, ellipsis.
- **Section** (600, 15px): side-panel headings.
- **Body** (400, 13px/1.45, -0.005em): all copy.
- **Label** (500, 13px): field labels, buttons, nav, file names.
- **Hint** (400, 12px, `dim`): helper text under fields, table headers, stage list, scene tags.
- **Data** (JetBrains Mono, tabular-nums): IDs, durations, mm, counts, percentages; stats at 500/15px, calculation readouts at 500/20px, scene dimensions at 500/11.5px.

### Named Rules
**The Tabular Rule.** Any number that can change or be compared is mono with `font-variant-numeric: tabular-nums`. Prose numbers ("24 ч") may stay in Plex.

**The No-Shouting Rule.** No uppercase labels, no letterspaced eyebrows, no display sizes above 22px. Hierarchy comes from weight and space.

## Layout

App shell: **topbar** 52px sticky, then a grid of **sidebar** 208px (collapses to a 56px icon **rail**; toggle at the left of the topbar) and content. Topbar right side holds only the theme icon and the account control.

Work screen (Goat Packer-specific): **form column 320px** (vertical form, fields stacked, groups separated by a top hairline and 14px padding), **main column** fluid (action bar, progress, result, scene), optional **side panel 340px** (AMS calibration steps). Column padding 18px; main column 18px 24px. Inner rhythm: 16px between blocks, 12px within a group, 5px label-to-control, 10px between paired fields.

Content pages (tables, job detail): centred, max-width 1100px, 24px padding, 16px gaps.

Responsive: at ≤1180px the side panel drops below the columns and the form narrows to 300px; at ≤760px the sidebar hides and the form stacks above the main column.

**The Same-Place Rule.** Progress and results render in the column where the job started. No navigation for status.

## Elevation & Depth

Flat by default. Depth is tonal: chrome is darker than bg, panels lighter (light theme) or one step up (dark), panel-2 recessed. Hairline borders separate regions. Shadows exist only for floating layers and the active segment.

### Shadow Vocabulary
- **Float** (`0 16px 48px rgb(0 0 0 / .12), 0 2px 8px rgb(0 0 0 / .08)`; dark `.5` / `.3`): menus, popovers.
- **Lift** (`0 1px 2px rgb(0 0 0 / .08)`; dark `.3`): primary button, active segmented option.

### Named Rules
**The Floating-Only Rule.** If it does not float above the page or is not the pressed choice, it has no shadow.

## Shapes

Three radii plus pill: 4px for inner pieces (segmented options, menu items, progress track), 6px for controls (buttons, inputs, nav items, notes), 10px for containers (cards, tables, file chip, drop zone, menu, scene, banners). Pill (999px) is for status pills and stub tags only; avatars are circles. Borders are 1px; the drop zone uses a dashed `border-strong` that turns `accent` on hover or drag-over.

Icons: 24px viewBox, stroke-only, `stroke-width: 1.75`, round caps and joins, `currentColor`, rendered at 16px (20px in banners). One SVG sprite referenced by `<use href="/icons.svg#name">` through a single `Icon` component (`aria-hidden`, label goes on the button).

## Components

### Buttons
- **Shape:** 6px corners; 32px default, 40px large (the main "Упаковать").
- **Primary:** `accent-fill` with `accent-ink`, no border, lift shadow. Hover: `brightness(1.06)`. Disabled: 45% opacity, `not-allowed`.
- **Secondary:** panel background, `border-strong` outline, text color; hover `hover`.
- **Icon button:** 32px square, transparent, `muted` icon; hover `hover` + `text`.
- **Link:** `accent` text, 500, underline on hover (offset 3px).
- **Focus (all):** 2px `accent` outline, 2px offset.

### Inputs / Fields
- **Style:** 32px, panel background, `border-strong`, 6px, 10px side padding. Label (500) above, hint (12px `dim`) below, 5px gap.
- **Focus:** border turns `accent`, plus a 3px `accent-soft` ring; no outline.
- **Suffix action:** password "показать" eye as an icon button inside the right edge.
- **Segmented control:** panel-2 track with 2px padding; options 26px, 12.5px/500, `muted`; selected option is panel + text + lift + 1px border ring.
- **Checkbox with nested reveal:** native checkbox in `accent`, label 500 with a 12px `dim` line under it; when checked, dependent fields appear indented 24px ("Учитывать реальную смену прутка" → "Смена прутка, с" + "Откалибровать заново…").
- **Disclosure:** 12px/600 `muted` text with a chevron that rotates 180° ("Дополнительно").

### File chip and drop zone
Drop zone: 10px dashed card, upload icon, "Перетащите 3MF или выберите файл". After upload it becomes a file chip: 10px card with file icon, name (500, ellipsis), meta line (12px `dim`, mono numbers) and a remove icon button.

### Progress (signature)
One 8px bar, segments proportional to each stage's share, 2px gaps. Done = `signal`; current = `signal` filled to its percentage over `border-strong`; pending = `border-strong` at 35%; skipped = diagonal hatching. Above: current stage in 600 and mono "идёт 1:12 · осталось ~3 мин". Below: stage list with 7px dots (current dot gets a 3px `accent-soft` halo; optional stages have dashed dots and "если понадобится"). Never show stage ids.

### Result banner
10px tonal card in `ok-soft` (or `crit-soft`), 20px status icon, title 600/14px ("Стол готов: 38 объектов"), muted line, then a stats row (12px `dim` label over 15px mono value) and actions. No border, no stripe.

### Pills
Pill radius, 12px/500, 6px currentColor dot: ok, running (`accent`), error, queued (panel-2 + muted). Status only.

### Tables
Panel background, 1px border, 10px corners. Header row panel-2, 12px/500 `dim`, 9px 14px. Rows 44px, hairline separators, `hover` on hover. ID column is a mono `accent` link to the detail page.

### Navigation
Sidebar items 34px, 6px corners, icon + label 500 in `muted`; hover `hover` + `text`; active `accent-soft` background, `text` label, `accent` icon. In rail mode labels hide and icons centre. Separator: 1px border with 8px margins.

### Menu
Panel, 1px border, 10px corners, float shadow, 6px padding, min-width 230px. Optional head (name 600, role 12px `dim`). Items 7px 10px, 4px corners, `dim` icons, `hover` on hover; hr separators.

### Auth card
Single column, max 380px, centred; heading 22px/600, muted subline, 14px field gap, full-width 40px primary, centred footer link. Stub features carry a `warn` pill ("скоро").

### Theme toggle
Icon button in the topbar: sun in dark theme, moon in light. Sets `data-theme="light|dark"` on `<html>` and persists to `localStorage`; first visit follows `prefers-color-scheme`. Apply before first paint to avoid a flash.

### Motion
Calm and functional: 150–200ms ease-out for hover, focus, disclosure and panel reveals. No bounce, no parallax, no animated layout entry.

### Scene card (Goat Packer-specific)
A 10px card in `scene-bg` with a 16:9.4 isometric/orthographic view of the real build plate: dark plate edge, light usable area with grid, keepouts, objects as flat-filled prisms of their real silhouettes and height. Corner tags (12px `dim`, values in `text` 500), legend top-right (12×9px swatches), dimension labels in mono 11.5px on a `scene-bg` chip. During a job the scene shows real stages (probe slice, layout, assembly from the centre, rising `signal` slice plane); with `prefers-reduced-motion: reduce` it shows a still frame of the current state. Results allow mouse rotation.

## Do's and Don'ts

### Do:
- **Do** keep orange to the primary action, progress and the slice plane; one primary button per view.
- **Do** use `#9c3d00` for orange text and icons on light, `#ff7a1a` only as a fill with `#1a0d00` ink.
- **Do** use green only for the projection contour, tower and success states.
- **Do** set every changing number in JetBrains Mono with tabular figures.
- **Do** separate regions with tonal steps and 1px hairlines; reserve shadows for menus and the pressed segment.
- **Do** write UI copy in Russian, plain and specific: "Упаковать", "Отменить", "Стол готов", "Откалибровать заново…"; hide internal jargon ("Фаза 1", `final_slice`).
- **Do** give every interactive element the 2px `accent` focus outline (inputs: accent border + 3px soft ring).
- **Do** keep transitions at 150–200ms ease-out and honor `prefers-reduced-motion`.
- **Do** draw icons as 24px stroke paths (1.75, round) in one sprite.

### Don't:
- **Don't** use pale cream backgrounds (the `#faf9f5` / `#f0eee6` family); light bg stays dense stone `#ebe7e2`.
- **Don't** use clay orange (the `#d97757` family) or any second orange.
- **Don't** use a serif or a display face anywhere.
- **Don't** use gradient text, eyebrow labels, uppercase letterspaced captions or decorative glow.
- **Don't** nest cards inside cards; a card holds rows, fields or a figure, not another card.
- **Don't** use colored `border-left` stripes for callouts or banners; use a tonal `-soft` background and an icon.
- **Don't** use emoji as icons or status markers.
- **Don't** use pills for anything except status and stub tags.
- **Don't** use `accent-2` green for categorical data (e.g. night bands in the schedule Gantt); use a neutral or hatching instead.
- **Don't** animate fake progress when real stage data exists.
