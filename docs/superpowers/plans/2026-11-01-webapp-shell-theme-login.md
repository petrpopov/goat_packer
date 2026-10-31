# Webapp shell, «Камень» theme and /login — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the two-row header with tabs by a one-row top bar + collapsible icon sidebar, real URL routing, the warm «Камень» palette with an icon theme toggle, an account menu, and a `/login` flow with front-end stubs for password reset by email.

**Architecture:** A tiny History-API router (pure `parseRoute`/`routePath` + a zustand slice) replaces the `activeTab` state; nginx (`deploy/nginx.conf: try_files $uri /index.html`) and Vite already fall back to `index.html`, so no backend change is needed. Pages that keep job state (Упаковка, Расписание, Калибровка) stay mounted and hidden, exactly as today; the jobs list/detail, admin and login render only on their routes. Styling stays plain CSS on the existing token names, so `core.css` keeps working with the new values.

**Tech Stack:** React 18, TypeScript, zustand, @tanstack/react-query, plain CSS, `bun test` for pure modules.

**Design sources (read first):**
- `DESIGN.md` (repo root) — normative tokens and component rules («Камень и искра»).
- Brief: `docs/superpowers/specs/2026-10-31-webapp-redesign-brief.md` — last section «Итоговая палитра: «Камень»».
- Approved mockup: `docs/superpowers/specs/2026-10-31-webapp-redesign-mockup-v2.html` (open in a browser; «Камень» is the default palette). Copy measurements from its CSS.

**Out of scope (other plans):** pack form rework, AMS calibration side panel, three.js scene (`2026-11-01-webapp-pack-workspace-scene.md`); packer `layout` event (`2026-11-01-packer-layout-event.md`).

All commands run from `webapp/frontend` unless stated otherwise.

---

## File structure

| File | Responsibility |
|---|---|
| `src/styles/tokens.css` (rewrite) | «Камень» light + warm-charcoal dark on the existing token names, plus `--accent-fill`, `--signal`, scene tokens |
| `src/styles/shell.css` (new) | top bar, sidebar/rail, account menu, auth card |
| `src/styles/core.css` (modify) | drop `.app-shell`, `.app-header*`, `.brand-dot`, `.topbar-spacer`, `.tabs`, `.tab*`; primary button on `--accent-fill`; `.ic` |
| `public/icons.svg` (new) | SVG sprite, 24px viewBox, stroke 1.75 (ozonware pattern) |
| `src/components/Icon.tsx` (new) | `<Icon name>` → `<use href="/icons.svg#name">` |
| `src/utils/routes.ts` (new) | pure `parseRoute(pathname)` / `routePath(route)` |
| `src/hooks/useLinkHandler.ts` (new) | client-side navigation for plain `<a href>` |
| `src/store/useUiStore.ts` (rewrite) | `route`, `navigate()`, `rail` (persisted), theme; `sendToSchedule` navigates |
| `src/components/Shell.tsx` (new) | top bar + sidebar + content slot |
| `src/components/AccountMenu.tsx` (new) | guest: «Войти» + menu; user: avatar menu |
| `src/utils/auth.ts` (new) | pure `authRequest(step, fields)` → endpoint + body |
| `src/pages/LoginPage.tsx` (new) | `/login`, `/login/register`, `/login/forgot`, `/login/sent`, `/login/code` |
| `src/App.tsx` (rewrite) | route → page |
| `src/pages/ServerPages.tsx` (modify) | delete `AccountPage`; `HistoryPage` selection from route |
| `index.html` (modify) | `<title>` |
| `tests/routes.test.ts`, `tests/auth.test.ts` (new) | unit tests |

---

### Task 1: «Камень» tokens

**Files:**
- Modify: `src/styles/tokens.css` (full rewrite)
- Modify: `src/styles/core.css` (`.btn-primary` blocks)

- [ ] **Step 1: Inventory token names used by CSS/TSX**

Run: `grep -rohE 'var\(--[a-z0-9-]+' src | sort -u`
Expected: every name is defined in Step 2. If one is missing, add it to the alias block mapped to the closest role.

- [ ] **Step 2: Rewrite `src/styles/tokens.css`**

```css
/* Палитра «Камень» (светлая) и тёплый уголь (тёмная) — см. DESIGN.md.
   Оранжевый — основной (действия и прогресс машины), зелёный — точечный акцент
   (контур проекции, башня, «готово»). Не кремовый фон и не глиняный оранжевый — намеренно. */
:root {
  font-family: 'IBM Plex Sans', -apple-system, system-ui, sans-serif;
  --font-ui: 'IBM Plex Sans', -apple-system, system-ui, sans-serif;
  --font-mono: 'JetBrains Mono', ui-monospace, Menlo, monospace;

  --bg: #ebe7e2;
  --bg-chrome: #e3ded8;
  --panel: #f8f6f3;
  --panel-2: #e0dbd5;
  --row: #f8f6f3;
  --row-alt: #f1eee9;
  --row-hover: #d9d3cc;
  --border: #cdc6be;
  --border-strong: #aba299;

  --fg: #1b1916;
  --fg-muted: #5a5249;
  --fg-dim: #5a5249;
  --fg-placeholder: #6b635a;

  /* Оранжевый текст/иконки — только #9c3d00 (5.5:1 на фоне); #ff7a1a — только заливка. */
  --accent: #9c3d00;
  --accent-fill: #ff7a1a;
  --accent-ink: #1a0d00;
  --accent-soft: #ff7a1a26;
  --signal: #ff7a1a;
  --accent-2: #006b4a;
  --accent-2-soft: #006b4a1f;

  --ok: #1f7a3a;
  --ok-soft: #1f7a3a1f;
  --warn: #9a5b00;
  --warn-soft: #9a5b001f;
  --crit: #c42847;
  --crit-soft: #c428471a;

  --shadow-lg: 0 16px 48px rgb(40 30 20 / 0.14), 0 2px 8px rgb(40 30 20 / 0.08);
  --shadow-sm: 0 1px 2px rgb(40 30 20 / 0.08);

  --radius-sm: 4px;
  --radius: 6px;
  --radius-lg: 10px;

  /* Сцена стола (план 2) */
  --scene-bg: #e4dfd9;
  --plate: #2b2724;
  --plate-top: #faf8f5;
  --plate-grid: #e2dcd4;
  --keepout: #8f877e;
  --object: #aea69c;

  /* Legacy aliases — core.css и компоненты используют эти имена */
  --bg-page: var(--bg);
  --bg-card: var(--panel);
  --bg-elev: var(--panel-2);
  --bg-hover: var(--row-hover);
  --bg-muted: var(--panel-2);
  --surface-alt: var(--panel-2);
  --text: var(--fg);
  --text-muted: var(--fg-muted);
  --text-placeholder: var(--fg-placeholder);
  --primary: var(--accent);
  --primary-contrast: var(--accent-ink);
  --danger: var(--crit);
  --danger-contrast: #ffffff;
  --color-success: var(--ok);
  --color-success-bg: var(--ok-soft);
  --color-warning: var(--warn);
  --color-warning-bg: var(--warn-soft);
  --color-danger: var(--crit);
  --color-danger-bg: var(--crit-soft);
  --color-muted: var(--fg-muted);
  --overlay-bg: rgb(0 0 0 / 0.5);
  --border-subtle: var(--border);
  --separator-bg: var(--border);
  --shadow-border: 0 0 0 1px var(--border);
  --shadow-border-light: 0 0 0 1px var(--row-hover);
  --shadow-card: 0 0 0 1px var(--border), 0 1px 2px rgb(40 30 20 / 0.05);
  --shadow-modal: var(--shadow-lg);
  --shadow-focus: 0 0 0 3px var(--accent-soft);
  --accent-hi: var(--accent-fill);
  --accent-lo: var(--accent-fill);
  --accent-edge-hi: transparent;
  --accent-glow: var(--accent-soft);
  --toast-fg: #ffffff;
  --toast-info-bg: #1b1916;
  --toast-error-bg: var(--crit);
  --toast-error-fg: #ffffff;

  --row-h: 44px;
  --z-toast: 1200;
  color: var(--text);
}

:root[data-theme='dark'] {
  --bg: #121110;
  --bg-chrome: #0f0e0d;
  --panel: #1b1918;
  --panel-2: #23201e;
  --row: #1b1918;
  --row-alt: #1f1d1b;
  --row-hover: #2a2724;
  --border: #34302c;
  --border-strong: #45403a;

  --fg: #ece8e3;
  --fg-muted: #a49d95;
  --fg-dim: #958e86;
  --fg-placeholder: #a49d95;

  --accent: #ff9640;
  --accent-fill: #ff9640;
  --accent-ink: #1a0d00;
  --accent-soft: #ff964024;
  --signal: #ff9640;
  --accent-2: #3ddc91;
  --accent-2-soft: #3ddc9124;

  --ok: #3ddc91;
  --ok-soft: #3ddc911f;
  --warn: #ffcd48;
  --warn-soft: #ffcd4824;
  --crit: #ff6285;
  --crit-soft: #ff628524;

  --shadow-lg: 0 16px 48px rgb(0 0 0 / 0.5), 0 2px 8px rgb(0 0 0 / 0.3);
  --shadow-sm: 0 1px 2px rgb(0 0 0 / 0.3);
  --shadow-card: 0 0 0 1px var(--border), 0 1px 2px rgb(0 0 0 / 0.3);

  --scene-bg: #161413;
  --plate: #070606;
  --plate-top: #221f1c;
  --plate-grid: #302c28;
  --keepout: #ece8e3;
  --object: #c2bbb3;

  --toast-fg: var(--fg);
  --toast-info-bg: #23201e;
  --toast-error-bg: #8f1d36;
}
```

Keep every rule that followed the dark block in the old file (body font settings, `.mono`, placeholder colour, etc.) — copy them unchanged after the new blocks.

- [ ] **Step 3: Primary button uses the fill token** — in `src/styles/core.css` replace the `.btn-primary`, `.btn-primary:hover:not(:disabled)`, `.btn-primary:active:not(:disabled)` blocks with:

```css
.btn-primary {
  border: 0;
  color: var(--accent-ink);
  background: var(--accent-fill);
  box-shadow: var(--shadow-sm);
}
.btn-primary:hover:not(:disabled) {
  filter: brightness(1.06);
}
.btn-primary:active:not(:disabled) {
  filter: brightness(0.96);
}
```

- [ ] **Step 4: Verify**

Run: `bun run typecheck && bun run lint && bun run build` → all succeed.
Run `bun run dev`, open http://localhost:3007, toggle the theme with the old button. Expected: light bg `#ebe7e2`, orange primary button with near-black text; dark bg `#121110`.

- [ ] **Step 5: Commit**

```bash
git add src/styles/tokens.css src/styles/core.css
git commit -m "Webapp: «Камень» palette (warm stone light, warm charcoal dark)"
```

---

### Task 2: Icon sprite

**Files:**
- Create: `public/icons.svg`, `src/components/Icon.tsx`
- Modify: `src/styles/core.css` (append `.ic`)

- [ ] **Step 1: Write `public/icons.svg`**

```xml
<svg xmlns="http://www.w3.org/2000/svg">
  <symbol id="sun" viewBox="0 0 24 24"><circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4"/></symbol>
  <symbol id="moon" viewBox="0 0 24 24"><path d="M20 14.5A8 8 0 0 1 9.5 4 8 8 0 1 0 20 14.5Z"/></symbol>
  <symbol id="menu" viewBox="0 0 24 24"><path d="M4 7h16M4 12h16M4 17h16"/></symbol>
  <symbol id="sidebar" viewBox="0 0 24 24"><rect x="3" y="4" width="18" height="16" rx="2"/><path d="M9 4v16"/></symbol>
  <symbol id="chevron-down" viewBox="0 0 24 24"><path d="m6 9 6 6 6-6"/></symbol>
  <symbol id="upload" viewBox="0 0 24 24"><path d="M12 15V4M7 9l5-5 5 5M5 20h14"/></symbol>
  <symbol id="x" viewBox="0 0 24 24"><path d="M6 6l12 12M18 6 6 18"/></symbol>
  <symbol id="check" viewBox="0 0 24 24"><circle cx="12" cy="12" r="9"/><path d="m8 12 3 3 5-6"/></symbol>
  <symbol id="alert" viewBox="0 0 24 24"><circle cx="12" cy="12" r="9"/><path d="M12 7.5v5.5M12 16.5v.01"/></symbol>
  <symbol id="download" viewBox="0 0 24 24"><path d="M12 4v11M7 10l5 5 5-5M5 20h14"/></symbol>
  <symbol id="arrow-right" viewBox="0 0 24 24"><path d="M5 12h14M13 6l6 6-6 6"/></symbol>
  <symbol id="eye" viewBox="0 0 24 24"><path d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7S2 12 2 12Z"/><circle cx="12" cy="12" r="3"/></symbol>
  <symbol id="eye-off" viewBox="0 0 24 24"><path d="M3 3l18 18M10.6 5.1A10.4 10.4 0 0 1 12 5c6.5 0 10 7 10 7a17.6 17.6 0 0 1-3.2 4.2M6.6 6.6C3.9 8.4 2 12 2 12s3.5 7 10 7c1.7 0 3.2-.4 4.5-1.1"/><path d="M9.9 9.9a3 3 0 0 0 4.2 4.2"/></symbol>
  <symbol id="list" viewBox="0 0 24 24"><path d="M9 6h11M9 12h11M9 18h11M4 6h.01M4 12h.01M4 18h.01"/></symbol>
  <symbol id="cube" viewBox="0 0 24 24"><path d="M12 3 20 7.5v9L12 21l-8-4.5v-9Z"/><path d="M4 7.5 12 12l8-4.5M12 12v9"/></symbol>
  <symbol id="calendar" viewBox="0 0 24 24"><rect x="3.5" y="5" width="17" height="15" rx="2"/><path d="M3.5 10h17M8 3v4M16 3v4"/></symbol>
  <symbol id="shield" viewBox="0 0 24 24"><path d="M12 3 5 6v6c0 4.5 3 7.5 7 9 4-1.5 7-4.5 7-9V6l-7-3Z"/></symbol>
  <symbol id="logout" viewBox="0 0 24 24"><path d="M15 4h4v16h-4M10 8l-4 4 4 4M6 12h10"/></symbol>
  <symbol id="user" viewBox="0 0 24 24"><circle cx="12" cy="8" r="4"/><path d="M4 21c1.5-4 4.5-6 8-6s6.5 2 8 6"/></symbol>
  <symbol id="file" viewBox="0 0 24 24"><path d="M14 3H6v18h12V7l-4-4Z"/><path d="M14 3v4h4"/></symbol>
  <symbol id="gauge" viewBox="0 0 24 24"><path d="M4 16a8 8 0 1 1 16 0"/><path d="m12 16 4-5"/></symbol>
</svg>
```

- [ ] **Step 2: Write `src/components/Icon.tsx`**

```tsx
/**
 * Иконки — один SVG-спрайт public/icons.svg (как в ozonware): 24px viewBox, обводка 1.75,
 * скруглённые концы. Новая иконка = новый <symbol> в спрайте, не эмодзи и не текстовый глиф.
 */
export type IconName =
  | 'sun' | 'moon' | 'menu' | 'sidebar' | 'chevron-down' | 'upload' | 'x' | 'check' | 'alert'
  | 'download' | 'arrow-right' | 'eye' | 'eye-off' | 'list' | 'cube' | 'calendar' | 'shield'
  | 'logout' | 'user' | 'file' | 'gauge';

export default function Icon({ name, size = 16 }: { name: IconName; size?: number }) {
  return (
    <svg className="ic" width={size} height={size} viewBox="0 0 24 24" aria-hidden="true">
      <use href={`/icons.svg#${name}`} />
    </svg>
  );
}
```

- [ ] **Step 3: Append to `src/styles/core.css`**

```css
.ic {
  flex: none;
  stroke: currentColor;
  fill: none;
  stroke-width: 1.75;
  stroke-linecap: round;
  stroke-linejoin: round;
}
```

- [ ] **Step 4: Verify** — `bun run typecheck && bun run lint && bun run build`; `ls dist/icons.svg` exists (Vite copies `public/`).

- [ ] **Step 5: Commit**

```bash
git add public/icons.svg src/components/Icon.tsx src/styles/core.css
git commit -m "Webapp: SVG icon sprite"
```

---

### Task 3: Routes (pure, TDD)

**Files:**
- Create: `src/utils/routes.ts`
- Test: `tests/routes.test.ts`

- [ ] **Step 1: Failing test `tests/routes.test.ts`**

```ts
import { describe, expect, test } from 'bun:test';
import { parseRoute, routePath } from '../src/utils/routes';

describe('parseRoute', () => {
  test('root and unknown paths open packing', () => {
    expect(parseRoute('/')).toEqual({ name: 'pack' });
    expect(parseRoute('/nope/x')).toEqual({ name: 'pack' });
  });
  test('sections', () => {
    expect(parseRoute('/pack')).toEqual({ name: 'pack' });
    expect(parseRoute('/schedule')).toEqual({ name: 'schedule' });
    expect(parseRoute('/calibration')).toEqual({ name: 'calibration' });
    expect(parseRoute('/jobs')).toEqual({ name: 'jobs' });
    expect(parseRoute('/admin')).toEqual({ name: 'admin' });
  });
  test('job detail keeps the share-link shape /jobs/:id', () => {
    expect(parseRoute('/jobs/7f3a9c-01')).toEqual({ name: 'job', id: '7f3a9c-01' });
    expect(parseRoute('/jobs/7f3a9c-01/')).toEqual({ name: 'job', id: '7f3a9c-01' });
  });
  test('login steps', () => {
    expect(parseRoute('/login')).toEqual({ name: 'login', step: 'signin' });
    expect(parseRoute('/login/register')).toEqual({ name: 'login', step: 'register' });
    expect(parseRoute('/login/forgot')).toEqual({ name: 'login', step: 'forgot' });
    expect(parseRoute('/login/sent')).toEqual({ name: 'login', step: 'sent' });
    expect(parseRoute('/login/code')).toEqual({ name: 'login', step: 'code' });
    expect(parseRoute('/login/other')).toEqual({ name: 'login', step: 'signin' });
  });
});

describe('routePath', () => {
  test('round-trips', () => {
    for (const path of ['/pack', '/schedule', '/calibration', '/jobs', '/jobs/abc', '/admin', '/login', '/login/code']) {
      expect(routePath(parseRoute(path))).toBe(path);
    }
  });
});
```

- [ ] **Step 2: Run** — `bun test tests/routes.test.ts` → FAIL (module not found).

- [ ] **Step 3: Implement `src/utils/routes.ts`**

```ts
/** Маршруты SPA. nginx и Vite отдают index.html на любой путь, поэтому это настоящие URL. */
export type LoginStep = 'signin' | 'register' | 'forgot' | 'sent' | 'code';

export type Route =
  | { name: 'pack' }
  | { name: 'schedule' }
  | { name: 'calibration' }
  | { name: 'jobs' }
  | { name: 'job'; id: string }
  | { name: 'admin' }
  | { name: 'login'; step: LoginStep };

const LOGIN_STEPS: LoginStep[] = ['register', 'forgot', 'sent', 'code'];
const JOB_ID = /^[a-z0-9-]+$/;

export function parseRoute(pathname: string): Route {
  const [head, tail] = pathname.split('/').filter(Boolean);
  switch (head) {
    case 'schedule':
      return { name: 'schedule' };
    case 'calibration':
      return { name: 'calibration' };
    case 'admin':
      return { name: 'admin' };
    case 'jobs':
      return tail && JOB_ID.test(tail) ? { name: 'job', id: tail } : { name: 'jobs' };
    case 'login':
      return { name: 'login', step: LOGIN_STEPS.find((step) => step === tail) ?? 'signin' };
    default:
      return { name: 'pack' };
  }
}

export function routePath(route: Route): string {
  switch (route.name) {
    case 'job':
      return `/jobs/${route.id}`;
    case 'login':
      return route.step === 'signin' ? '/login' : `/login/${route.step}`;
    default:
      return `/${route.name}`;
  }
}
```

- [ ] **Step 4: Run** — `bun test tests/routes.test.ts` → PASS.

- [ ] **Step 5: Commit**

```bash
git add src/utils/routes.ts tests/routes.test.ts
git commit -m "Webapp: URL routes (parseRoute/routePath)"
```

---

### Task 4: UI store with navigation and rail; link handler

**Files:**
- Rewrite: `src/store/useUiStore.ts`
- Create: `src/hooks/useLinkHandler.ts`

- [ ] **Step 1: Rewrite `src/store/useUiStore.ts`**

```ts
import { create } from 'zustand';
import { parseRoute, type Route } from '@/utils/routes';

export type Theme = 'dark' | 'light';

export interface PackHandoff {
  packJobId: string;
  profileId: string;
}

const THEME_KEY = 'goat-packer-theme';
const RAIL_KEY = 'goat-packer-rail';

function readStored(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}
function writeStored(key: string, value: string): void {
  try {
    localStorage.setItem(key, value);
  } catch {
    /* приватный режим — просто не сохранится */
  }
}

function readInitialTheme(): Theme {
  return readStored(THEME_KEY) === 'light' ? 'light' : 'dark';
}
function applyTheme(theme: Theme): void {
  document.documentElement.dataset.theme = theme;
}

/** Один раз до рендера: тема без мигания + синхронизация маршрута с кнопками «назад/вперёд». */
export function initTheme(): void {
  applyTheme(readInitialTheme());
  window.addEventListener('popstate', () => useUiStore.setState({ route: parseRoute(location.pathname) }));
}

interface UiState {
  theme: Theme;
  route: Route;
  /** Сайдбар свёрнут в полосу иконок. */
  rail: boolean;
  /** Результат упаковки, переданный в расписание («В расписание»). */
  packHandoff: PackHandoff | null;
  toggleTheme: () => void;
  toggleRail: () => void;
  navigate: (path: string, options?: { replace?: boolean }) => void;
  sendToSchedule: (handoff: PackHandoff) => void;
}

export const useUiStore = create<UiState>((set, get) => ({
  theme: readInitialTheme(),
  route: parseRoute(typeof location === 'undefined' ? '/' : location.pathname),
  rail: readStored(RAIL_KEY) === '1',
  packHandoff: null,
  toggleTheme: () => {
    const next: Theme = get().theme === 'dark' ? 'light' : 'dark';
    applyTheme(next);
    writeStored(THEME_KEY, next);
    set({ theme: next });
  },
  toggleRail: () => {
    const next = !get().rail;
    writeStored(RAIL_KEY, next ? '1' : '0');
    set({ rail: next });
  },
  navigate: (path, options) => {
    if (path !== location.pathname) {
      if (options?.replace) history.replaceState(null, '', path);
      else history.pushState(null, '', path);
    }
    set({ route: parseRoute(path) });
    window.scrollTo(0, 0);
  },
  sendToSchedule: (handoff) => {
    set({ packHandoff: handoff });
    get().navigate('/schedule');
  },
}));
```

- [ ] **Step 2: Write `src/hooks/useLinkHandler.ts`**

```ts
import type { MouseEvent } from 'react';
import { useUiStore } from '@/store/useUiStore';

/**
 * Внутренние ссылки — обычные <a href>: средняя кнопка и «открыть в новой вкладке» работают,
 * обычный клик переходит без перезагрузки страницы.
 */
export function useLinkHandler() {
  const navigate = useUiStore((s) => s.navigate);
  return (event: MouseEvent<HTMLAnchorElement>) => {
    if (event.defaultPrevented || event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
    event.preventDefault();
    navigate(event.currentTarget.getAttribute('href') ?? '/');
  };
}
```

- [ ] **Step 3: Typecheck lists the call sites to fix** — `bun run typecheck` → errors in `App.tsx`, `pages/ServerPages.tsx` (`activeTab`, `setTab`). They are fixed in Tasks 5–8; no commit yet.

---

### Task 5: Shell (top bar + sidebar)

**Files:**
- Create: `src/components/Shell.tsx`, `src/styles/shell.css`
- Modify: `src/main.tsx`, `src/styles/core.css`, `index.html`

- [ ] **Step 1: Write `src/components/Shell.tsx`**

```tsx
import type { ReactNode } from 'react';
import { serverMode } from '@/api/http';
import AccountMenu from '@/components/AccountMenu';
import Icon, { type IconName } from '@/components/Icon';
import { useLinkHandler } from '@/hooks/useLinkHandler';
import type { Session } from '@/pages/ServerPages';
import { useUiStore } from '@/store/useUiStore';
import type { Route } from '@/utils/routes';

interface NavItem {
  path: string;
  label: string;
  icon: IconName;
  match: Route['name'][];
}

const WORK_ITEMS: NavItem[] = [
  { path: '/pack', label: 'Упаковка', icon: 'cube', match: ['pack'] },
  { path: '/schedule', label: 'Расписание', icon: 'calendar', match: ['schedule'] },
  // Временно: план 2 переносит калибровку в боковую панель упаковки и удаляет этот пункт.
  { path: '/calibration', label: 'Калибровка AMS', icon: 'gauge', match: ['calibration'] },
];
const JOBS_ITEM: NavItem = { path: '/jobs', label: 'Мои задачи', icon: 'list', match: ['jobs', 'job'] };
const ADMIN_ITEM: NavItem = { path: '/admin', label: 'Администрирование', icon: 'shield', match: ['admin'] };

export default function Shell({ user, children }: { user?: Session; children: ReactNode }) {
  const route = useUiStore((s) => s.route);
  const rail = useUiStore((s) => s.rail);
  const toggleRail = useUiStore((s) => s.toggleRail);
  const theme = useUiStore((s) => s.theme);
  const toggleTheme = useUiStore((s) => s.toggleTheme);
  const onLink = useLinkHandler();

  const navLink = (item: NavItem) => {
    const active = item.match.includes(route.name);
    return (
      <a
        key={item.path}
        href={item.path}
        onClick={onLink}
        title={item.label}
        className={`nav-item${active ? ' active' : ''}`}
        aria-current={active ? 'page' : undefined}
      >
        <Icon name={item.icon} />
        <span className="nav-label">{item.label}</span>
      </a>
    );
  };

  return (
    <div className={`app${rail ? ' rail' : ''}`}>
      <header className="topbar">
        <button
          type="button"
          className="icon-btn"
          onClick={toggleRail}
          aria-pressed={rail}
          aria-label={rail ? 'Развернуть меню' : 'Свернуть меню'}
          title={rail ? 'Развернуть меню' : 'Свернуть меню'}
        >
          <Icon name="sidebar" />
        </button>
        <a className="brand" href="/pack" onClick={onLink}>
          <svg viewBox="0 0 24 24" width="22" height="22" aria-hidden="true">
            <path d="M12 3 20 7.5v9L12 21l-8-4.5v-9Z" fill="var(--accent-soft)" stroke="var(--accent-fill)" strokeWidth="1.6" strokeLinejoin="round" />
            <path d="M4 7.5 12 12l8-4.5M12 12v9" fill="none" stroke="var(--accent-fill)" strokeWidth="1.6" strokeLinejoin="round" />
          </svg>
          Goat Packer
        </a>
        <div className="topbar-spacer" />
        <button
          type="button"
          className="icon-btn"
          onClick={toggleTheme}
          aria-label="Переключить тему"
          title={theme === 'dark' ? 'Светлая тема' : 'Тёмная тема'}
        >
          <Icon name={theme === 'dark' ? 'sun' : 'moon'} />
        </button>
        {serverMode && <AccountMenu user={user} />}
      </header>
      <div className="app-body">
        <nav className="sidebar" aria-label="Разделы">
          {WORK_ITEMS.map(navLink)}
          {serverMode && navLink(JOBS_ITEM)}
          {user?.role === 'ADMIN' && (
            <>
              <div className="nav-sep" />
              {navLink(ADMIN_ITEM)}
            </>
          )}
        </nav>
        <main className="app-content">{children}</main>
      </div>
    </div>
  );
}
```

- [ ] **Step 2: Write `src/styles/shell.css`** (values from the mockup)

```css
.app { min-height: 100vh; }
.topbar {
  position: sticky; top: 0; z-index: 30;
  height: 52px; display: flex; align-items: center; gap: 10px;
  padding: 0 16px 0 10px;
  background: var(--bg-chrome); border-bottom: 1px solid var(--border);
}
.brand {
  display: flex; align-items: center; gap: 9px;
  font-weight: 600; font-size: 14px; letter-spacing: -0.01em;
  color: var(--fg); text-decoration: none;
}
.topbar-spacer { flex: 1; }
.icon-btn {
  width: 32px; height: 32px; display: inline-grid; place-items: center;
  border: 0; border-radius: var(--radius); background: transparent; color: var(--fg-muted); cursor: pointer;
}
.icon-btn:hover { background: var(--row-hover); color: var(--fg); }
.icon-btn:focus-visible, .nav-item:focus-visible, .brand:focus-visible, .avatar:focus-visible {
  outline: 2px solid var(--accent); outline-offset: 2px;
}

.app-body { display: grid; grid-template-columns: 208px minmax(0, 1fr); min-height: calc(100vh - 52px); }
.app.rail .app-body { grid-template-columns: 56px minmax(0, 1fr); }
.sidebar {
  position: sticky; top: 52px; height: calc(100vh - 52px);
  display: flex; flex-direction: column; gap: 2px; padding: 10px 8px;
  background: var(--bg-chrome); border-right: 1px solid var(--border);
}
.nav-item {
  display: flex; align-items: center; gap: 11px; height: 34px; padding: 0 11px;
  border-radius: var(--radius); color: var(--fg-muted); text-decoration: none; font-weight: 500;
  white-space: nowrap; overflow: hidden;
}
.nav-item:hover { background: var(--row-hover); color: var(--fg); }
.nav-item.active { background: var(--accent-soft); color: var(--fg); }
.nav-item.active .ic { color: var(--accent); }
.nav-sep { height: 1px; background: var(--border); margin: 8px 4px; }
.app.rail .nav-label { display: none; }
.app.rail .nav-item { justify-content: center; padding: 0; }
.app-content { min-width: 0; }

/* Меню аккаунта */
.menu-wrap { position: relative; }
.avatar {
  width: 30px; height: 30px; border-radius: 50%; border: 0; cursor: pointer;
  display: inline-grid; place-items: center;
  background: var(--accent-soft); color: var(--accent); font-weight: 600; font-size: 12px;
}
.menu {
  position: absolute; right: 0; top: calc(100% + 6px); z-index: 40; min-width: 230px; padding: 6px;
  background: var(--panel); border: 1px solid var(--border); border-radius: var(--radius-lg); box-shadow: var(--shadow-lg);
}
.menu-head { padding: 8px 10px 10px; margin-bottom: 6px; border-bottom: 1px solid var(--border); }
.menu-head b { display: block; font-weight: 600; }
.menu-head span { color: var(--fg-dim); font-size: 12px; }
.menu-item {
  display: flex; align-items: center; gap: 10px; width: 100%; padding: 7px 10px;
  border: 0; border-radius: var(--radius-sm); background: none; color: var(--fg);
  font: inherit; text-align: left; text-decoration: none; cursor: pointer;
}
.menu-item:hover, .menu-item:focus-visible { background: var(--row-hover); outline: none; }
.menu-item .ic { color: var(--fg-dim); }
.menu-sep { height: 1px; background: var(--border); margin: 6px 0; }

/* Вход */
.auth { min-height: calc(100vh - 52px); display: grid; place-items: center; padding: 32px 16px; }
.auth-card { width: 100%; max-width: 380px; display: grid; gap: 18px; }
.auth-card h1 { margin: 0; font-size: 22px; font-weight: 600; letter-spacing: -0.02em; }
.auth-sub { margin: -10px 0 0; color: var(--fg-muted); }
.auth-card form { display: grid; gap: 14px; }
.auth-foot { margin: 0; text-align: center; color: var(--fg-muted); }
.auth-note { padding: 10px 12px; border-radius: var(--radius); background: var(--panel-2); color: var(--fg-muted); font-size: 12.5px; }
.stub-badge { display: inline-block; padding: 1px 7px; border-radius: 999px; background: var(--warn-soft); color: var(--warn); font-size: 11px; font-weight: 600; }
.input-suffix { position: relative; }
.input-suffix .input { padding-right: 36px; }
.input-suffix .icon-btn { position: absolute; right: 1px; top: 1px; }
.label-row { display: flex; justify-content: space-between; align-items: baseline; }
.btn-block { width: 100%; }
.btn-lg { height: 40px; padding: 0 20px; font-size: 14px; }
.link { color: var(--accent); font-weight: 500; text-decoration: none; background: none; border: 0; padding: 0; font: inherit; cursor: pointer; }
.link:hover { text-decoration: underline; text-underline-offset: 3px; }

@media (max-width: 760px) {
  .app-body, .app.rail .app-body { grid-template-columns: 1fr; }
  .sidebar { position: static; height: auto; flex-direction: row; overflow-x: auto; border-right: 0; border-bottom: 1px solid var(--border); }
  .nav-sep { display: none; }
}
```

- [ ] **Step 3: Wire CSS and title**
  - `src/main.tsx`: add `import './styles/shell.css';` after `import './styles/core.css';`.
  - `index.html`: `<title>Goat Packer — упаковщик 3D-объектов для печати с максимальной плотностью</title>`.
  - `src/styles/core.css`: delete the blocks `.app-shell`, `.app-header`, `.app-header-brand`, `.brand-dot`, `.topbar-spacer`, `.tabs`, `.tab`, `.tab:hover`, `.tab.active`.

---

### Task 6: Account menu

**Files:**
- Create: `src/components/AccountMenu.tsx`

- [ ] **Step 1: Write `src/components/AccountMenu.tsx`**

```tsx
import { useEffect, useRef, useState } from 'react';
import type { MouseEvent } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { serverApi } from '@/api/http';
import Icon from '@/components/Icon';
import { useLinkHandler } from '@/hooks/useLinkHandler';
import type { Session } from '@/pages/ServerPages';
import { pushToast } from '@/store/useToastStore';
import { useUiStore } from '@/store/useUiStore';

/** «petr» → «PE», «petr.popov» → «PP». */
export function initialsOf(login: string): string {
  const words = login.trim().split(/[\s._-]+/).filter(Boolean);
  return (words.length >= 2 ? words[0][0] + words[1][0] : login.slice(0, 2)).toUpperCase();
}

/**
 * Справа в шапке. Гость: «Войти» + меню (гостевая сессия, Мои задачи, Создать аккаунт).
 * Пользователь: аватар-меню (Мои задачи, Администрирование для админа, Выйти).
 * Сброса пароля здесь нет — только на /login. Закрытие: клик вне, Escape (фокус на триггер).
 */
export default function AccountMenu({ user }: { user?: Session }) {
  const [open, setOpen] = useState(false);
  const wrapRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const onLink = useLinkHandler();
  const navigate = useUiStore((s) => s.navigate);
  const queryClient = useQueryClient();

  useEffect(() => {
    if (!open) return;
    const onPointer = (e: PointerEvent) => {
      if (!wrapRef.current?.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        setOpen(false);
        triggerRef.current?.focus();
      }
    };
    document.addEventListener('pointerdown', onPointer);
    document.addEventListener('keydown', onKey);
    return () => {
      document.removeEventListener('pointerdown', onPointer);
      document.removeEventListener('keydown', onKey);
    };
  }, [open]);

  const guest = !user || user.kind === 'GUEST';
  const go = (event: MouseEvent<HTMLAnchorElement>) => {
    setOpen(false);
    onLink(event);
  };
  const logout = async () => {
    setOpen(false);
    try {
      await serverApi.post('/auth/logout');
      await queryClient.invalidateQueries({ queryKey: ['session'] });
      navigate('/pack');
    } catch (e) {
      pushToast(e instanceof Error ? e.message : 'Не удалось выйти', 'error');
    }
  };

  return (
    <>
      {guest && (
        <a className="btn" href="/login" onClick={onLink}>
          Войти
        </a>
      )}
      <div className="menu-wrap" ref={wrapRef}>
        <button
          ref={triggerRef}
          type="button"
          className={guest ? 'icon-btn' : 'avatar'}
          aria-haspopup="menu"
          aria-expanded={open}
          aria-label={guest ? 'Меню' : `Аккаунт: ${user.login ?? ''}`}
          title={guest ? 'Меню' : (user.login ?? '')}
          onClick={() => setOpen((v) => !v)}
        >
          {guest ? <Icon name="menu" /> : initialsOf(user.login ?? '?')}
        </button>
        {open && (
          <div className="menu" role="menu">
            <div className="menu-head">
              <b>{guest ? 'Гостевая сессия' : user.login}</b>
              <span>{guest ? 'результаты хранятся 24 часа' : user.role === 'ADMIN' ? 'администратор' : 'аккаунт'}</span>
            </div>
            <a className="menu-item" role="menuitem" href="/jobs" onClick={go}>
              <Icon name="list" />
              Мои задачи
            </a>
            {guest ? (
              <a className="menu-item" role="menuitem" href="/login/register" onClick={go}>
                <Icon name="user" />
                Создать аккаунт
              </a>
            ) : (
              <>
                {user.role === 'ADMIN' && (
                  <a className="menu-item" role="menuitem" href="/admin" onClick={go}>
                    <Icon name="shield" />
                    Администрирование
                  </a>
                )}
                <div className="menu-sep" role="separator" />
                <button type="button" className="menu-item" role="menuitem" onClick={() => void logout()}>
                  <Icon name="logout" />
                  Выйти
                </button>
              </>
            )}
          </div>
        )}
      </div>
    </>
  );
}
```

---

### Task 7: Auth requests (pure, TDD) and /login page

**Files:**
- Create: `src/utils/auth.ts`, `src/pages/LoginPage.tsx`
- Test: `tests/auth.test.ts`

- [ ] **Step 1: Failing test `tests/auth.test.ts`**

```ts
import { describe, expect, test } from 'bun:test';
import { authRequest, PASSWORD_MIN } from '../src/utils/auth';

describe('authRequest', () => {
  test('sign in and register send login + password to the existing endpoints', () => {
    expect(authRequest('signin', { login: ' petr ', password: 'x'.repeat(12) })).toEqual({
      path: '/auth/login',
      body: { login: 'petr', password: 'x'.repeat(12) },
    });
    expect(authRequest('register', { login: 'petr', password: 'y'.repeat(12), email: 'a@b.c' })).toEqual({
      path: '/auth/register',
      body: { login: 'petr', password: 'y'.repeat(12) },
    });
  });
  test('admin code resets the password', () => {
    expect(authRequest('code', { token: ' t0k ', password: 'z'.repeat(12) })).toEqual({
      path: '/auth/reset',
      body: { token: 't0k', password: 'z'.repeat(12) },
    });
  });
  test('email reset is a front-end stub: no request', () => {
    expect(authRequest('forgot', { login: 'petr' })).toBeNull();
    expect(authRequest('sent', {})).toBeNull();
  });
  test('password rule is 12+ characters', () => {
    expect(PASSWORD_MIN).toBe(12);
  });
});
```

- [ ] **Step 2: Run** — `bun test tests/auth.test.ts` → FAIL (module missing).

- [ ] **Step 3: Implement `src/utils/auth.ts`**

```ts
import type { LoginStep } from './routes';

/** Бэкенд требует пароль 12–72 символа (те же лимиты, что были в AccountPage). */
export const PASSWORD_MIN = 12;
export const PASSWORD_MAX = 72;

export interface AuthFields {
  login?: string;
  password?: string;
  token?: string;
  /** Поле-заготовка регистрации: бэкенд email пока не принимает, не отправляется. */
  email?: string;
}

/**
 * Запрос к /api/v2/auth/* для шага формы. null — заготовка без сервера: письма для сброса
 * пароля пока не отправляются, экран «Проверьте почту» показывается без запроса.
 */
export function authRequest(step: LoginStep, fields: AuthFields): { path: string; body: Record<string, string> } | null {
  const password = fields.password ?? '';
  switch (step) {
    case 'signin':
      return { path: '/auth/login', body: { login: (fields.login ?? '').trim(), password } };
    case 'register':
      return { path: '/auth/register', body: { login: (fields.login ?? '').trim(), password } };
    case 'code':
      return { path: '/auth/reset', body: { token: (fields.token ?? '').trim(), password } };
    default:
      return null;
  }
}
```

- [ ] **Step 4: Run** — `bun test tests/auth.test.ts` → PASS.

- [ ] **Step 5: Write `src/pages/LoginPage.tsx`**

```tsx
import { useState } from 'react';
import type { FormEvent, ReactNode } from 'react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { serverApi } from '@/api/http';
import Icon from '@/components/Icon';
import { useLinkHandler } from '@/hooks/useLinkHandler';
import { useUiStore } from '@/store/useUiStore';
import { authRequest, PASSWORD_MAX, PASSWORD_MIN } from '@/utils/auth';
import type { LoginStep } from '@/utils/routes';

function PasswordField(props: {
  label: string;
  autoComplete: 'current-password' | 'new-password';
  value: string;
  onChange: (value: string) => void;
  aside?: ReactNode;
}) {
  const [shown, setShown] = useState(false);
  const isNew = props.autoComplete === 'new-password';
  return (
    <div className="field">
      <span className="field-label label-row">
        <label htmlFor="pw">{props.label}</label>
        {props.aside}
      </span>
      <div className="input-suffix">
        <input
          id="pw"
          className="input"
          type={shown ? 'text' : 'password'}
          autoComplete={props.autoComplete}
          value={props.value}
          onChange={(e) => props.onChange(e.target.value)}
          required
          minLength={isNew ? PASSWORD_MIN : undefined}
          maxLength={PASSWORD_MAX}
        />
        <button type="button" className="icon-btn" onClick={() => setShown((v) => !v)} aria-label={shown ? 'Скрыть пароль' : 'Показать пароль'}>
          <Icon name={shown ? 'eye-off' : 'eye'} />
        </button>
      </div>
      {isNew && <span className="field-hint">минимум {PASSWORD_MIN} символов</span>}
    </div>
  );
}

/**
 * Вход по паттерну web.dev: одна карточка, «Забыли пароль?» у поля пароля, показ пароля вместо
 * повтора, разные подписи кнопок («Войти» / «Создать аккаунт»), введённое не стирается при ошибке.
 * Email и письма — заготовки: бэкенд их пока не поддерживает (сброс — кодом от администратора).
 */
export default function LoginPage({ step }: { step: LoginStep }) {
  const [login, setLogin] = useState('');
  const [password, setPassword] = useState('');
  const [email, setEmail] = useState('');
  const [token, setToken] = useState('');
  const [error, setError] = useState<string | null>(null);
  const navigate = useUiStore((s) => s.navigate);
  const onLink = useLinkHandler();
  const queryClient = useQueryClient();

  const mutation = useMutation({
    mutationFn: async () => {
      const request = authRequest(step, { login, password, token, email });
      if (request) await serverApi.post(request.path, request.body);
    },
    onSuccess: async () => {
      setPassword('');
      await queryClient.invalidateQueries({ queryKey: ['session'] });
      navigate('/pack');
    },
    onError: (e) => setError(e instanceof Error ? e.message : 'Не удалось выполнить запрос'),
  });

  const submit = (e: FormEvent) => {
    e.preventDefault();
    setError(null);
    if (step === 'forgot') navigate('/login/sent');
    else mutation.mutate();
  };
  const link = (href: string, text: string) => (
    <a className="link" href={href} onClick={onLink}>
      {text}
    </a>
  );
  const back = <p className="auth-foot">{link('/login', 'Вернуться ко входу')}</p>;
  const loginField = (label: string) => (
    <div className="field">
      <label className="field-label" htmlFor="lg">{label}</label>
      <input id="lg" className="input" autoComplete="username" value={login} onChange={(e) => setLogin(e.target.value)} required minLength={3} maxLength={120} />
    </div>
  );
  const submitButton = (text: string) => (
    <button className="btn btn-primary btn-lg btn-block" disabled={mutation.isPending}>
      {text}
    </button>
  );
  const errorBlock = error ? <div className="error-block" role="alert">{error}</div> : null;

  let body: ReactNode;
  if (step === 'register') {
    body = (
      <>
        <h1>Создать аккаунт</h1>
        <p className="auth-sub">Задачи будут храниться без ограничения 24 часа</p>
        <form onSubmit={submit}>
          {loginField('Логин')}
          <div className="field">
            <label className="field-label" htmlFor="em">
              Email <span className="field-hint">необязательно</span>
            </label>
            <input id="em" className="input" type="email" autoComplete="email" value={email} onChange={(e) => setEmail(e.target.value)} />
            <span className="field-hint">
              для восстановления пароля <span className="stub-badge">скоро</span>
            </span>
          </div>
          <PasswordField label="Пароль" autoComplete="new-password" value={password} onChange={setPassword} />
          {errorBlock}
          {submitButton('Создать аккаунт')}
        </form>
        <div className="auth-note">Ваши гостевые задачи перейдут в новый аккаунт.</div>
        <p className="auth-foot">Уже есть аккаунт? {link('/login', 'Войти')}</p>
      </>
    );
  } else if (step === 'forgot') {
    body = (
      <>
        <h1>Восстановление пароля</h1>
        <p className="auth-sub">Пришлём ссылку для нового пароля на email аккаунта</p>
        <form onSubmit={submit}>
          {loginField('Логин или email')}
          {submitButton('Отправить ссылку')}
        </form>
        <div className="auth-note">Есть одноразовый код от администратора? {link('/login/code', 'Ввести код')}</div>
        {back}
      </>
    );
  } else if (step === 'sent') {
    body = (
      <>
        <h1>Проверьте почту</h1>
        <p className="auth-sub">Если у аккаунта указан email, ссылка уже в пути. Она действует 1 час.</p>
        <div className="auth-note">
          <span className="stub-badge">заготовка</span> Письма пока не отправляются. Сейчас пароль сбрасывает администратор — {link('/login/code', 'ввести его код')}.
        </div>
        {back}
      </>
    );
  } else if (step === 'code') {
    body = (
      <>
        <h1>Новый пароль</h1>
        <p className="auth-sub">Код выдаёт администратор, он одноразовый</p>
        <form onSubmit={submit}>
          <div className="field">
            <label className="field-label" htmlFor="tk">Код</label>
            <input id="tk" className="input mono" autoComplete="one-time-code" value={token} onChange={(e) => setToken(e.target.value)} required />
          </div>
          <PasswordField label="Новый пароль" autoComplete="new-password" value={password} onChange={setPassword} />
          {errorBlock}
          {submitButton('Сохранить и войти')}
        </form>
        {back}
      </>
    );
  } else {
    body = (
      <>
        <h1>Вход</h1>
        <p className="auth-sub">Goat Packer — упаковка 3D-объектов для печати</p>
        <form onSubmit={submit}>
          {loginField('Логин')}
          <PasswordField
            label="Пароль"
            autoComplete="current-password"
            value={password}
            onChange={setPassword}
            aside={
              <a className="link" style={{ fontSize: 12 }} href="/login/forgot" onClick={onLink}>
                Забыли пароль?
              </a>
            }
          />
          {errorBlock}
          {submitButton('Войти')}
        </form>
        <p className="auth-foot">Нет аккаунта? {link('/login/register', 'Создать аккаунт')}</p>
        <div className="auth-note">Можно работать и без входа: гостевые задачи хранятся 24 часа. {link('/pack', 'К упаковке')}</div>
      </>
    );
  }
  return (
    <div className="auth">
      <div className="auth-card">{body}</div>
    </div>
  );
}
```

Note: the old `AccountPage` reloaded the page after `/auth/reset`, so the backend may or may not sign the user in. If the manual check in Task 8 shows a guest session after a code reset, make `onSuccess` for `step === 'code'` call `pushToast('Пароль изменён, войдите', 'success')` and `navigate('/login')` instead of `/pack`.

---

### Task 8: App wiring, remove AccountPage, history follows the URL

**Files:**
- Rewrite: `src/App.tsx`
- Modify: `src/pages/ServerPages.tsx`

- [ ] **Step 1: Rewrite `src/App.tsx`**

```tsx
import { serverMode } from '@/api/http';
import { AdminPage, HistoryPage, ServerGate, useSession } from '@/pages/ServerPages';
import type { Session } from '@/pages/ServerPages';
import Shell from '@/components/Shell';
import ToastHost from '@/components/ToastHost';
import CalibrationPage from '@/pages/CalibrationPage';
import LoginPage from '@/pages/LoginPage';
import PackPage from '@/pages/PackPage';
import SchedulePage from '@/pages/SchedulePage';
import { useUiStore } from '@/store/useUiStore';

export default function App() {
  return serverMode ? (
    <ServerGate>
      <ServerApplication />
    </ServerGate>
  ) : (
    <Workspace />
  );
}

function ServerApplication() {
  const session = useSession();
  return <Workspace user={session.data} />;
}

function Workspace({ user }: { user?: Session }) {
  const route = useUiStore((s) => s.route);
  // Локальный режим без аккаунтов: история, вход и админка открывают упаковку.
  const serverOnly = route.name === 'jobs' || route.name === 'job' || route.name === 'login' || route.name === 'admin';
  const visible = (!serverMode && serverOnly) || (route.name === 'admin' && user?.role !== 'ADMIN') ? 'pack' : route.name;

  return (
    <Shell user={user}>
      {/* Рабочие страницы остаются смонтированными: идущая задача и форма не теряются при переходах. */}
      <div hidden={visible !== 'pack'}>
        <PackPage />
      </div>
      <div hidden={visible !== 'schedule'}>
        <SchedulePage />
      </div>
      <div hidden={visible !== 'calibration'}>
        <CalibrationPage />
      </div>
      {serverMode && (visible === 'jobs' || visible === 'job') && <HistoryPage />}
      {serverMode && route.name === 'login' && <LoginPage step={route.step} />}
      {serverMode && visible === 'admin' && <AdminPage />}
      <ToastHost />
    </Shell>
  );
}
```

- [ ] **Step 2: `ServerPages.tsx` — delete `AccountPage`** (the whole `export function AccountPage() { … }` block, currently lines ~55–145) and any imports lint then reports as unused.

- [ ] **Step 3: `HistoryPage` selection follows the URL**
  - Replace the `const [selected, setSelected] = useState<string | null>(…)` declaration (its initializer reads the job id from `location.pathname`) with:

```tsx
  const route = useUiStore((s) => s.route);
  const navigate = useUiStore((s) => s.navigate);
  const selected = route.name === 'job' ? route.id : null;
  const setSelected = (id: string | null) => navigate(id ? `/jobs/${id}` : '/jobs');
```

  - Delete `const setTab = useUiStore((s) => s.setTab);` and replace `onClick={() => setTab('pack')}` with `onClick={() => navigate('/pack')}`.
  - In the jobs table row, render the id cell as a link (find where the row prints the job id):

```tsx
<a
  className="mono link"
  href={`/jobs/${job.jobId}`}
  onClick={(e) => {
    e.preventDefault();
    setSelected(job.jobId);
  }}
>
  {job.jobId.slice(0, 8)}
</a>
```

- [ ] **Step 4: Full verification**

Run: `bun run typecheck && bun run lint && bun test tests && bun run build`
Expected: all pass (existing tests + `routes`, `auth`).

Manual, local mode (`bun run dev`):
- one-row top bar, no «Фаза»; sidebar «Упаковка / Расписание / Калибровка AMS»; theme icon toggles and survives reload; rail toggle survives reload;
- back/forward switch sections; a running pack job survives a trip to «Расписание» and back.

Manual, server mode (`VITE_SERVER_MODE=true bun run dev` + backend `./gradlew bootRun` with server profile per `deploy/README.md`):
- guest: «Войти» + ☰ (Гостевая сессия / Мои задачи / Создать аккаунт), no password reset in the menu;
- `/login`: wrong password shows the error and keeps the login; «Забыли пароль?» → `/login/forgot` → «Отправить ссылку» → `/login/sent` (stub badge); «Ввести код» → `/login/code`;
- register → avatar menu (Мои задачи, Выйти); admin also sees «Администрирование» in the sidebar and menu;
- `/jobs/<id>` opened directly (share link) shows the job; clicking an id in «Мои задачи» changes the URL.

- [ ] **Step 5: Commit**

```bash
git add -A src tests index.html public
git commit -m "Webapp: one-row top bar, icon sidebar, URL routing, account menu, /login with reset stubs"
```

---

## Self-review
- Brief §1 (header «Goat Packer», title text, account menu without reset) → Tasks 5, 6; navigation in a collapsible icon sidebar → Task 5; `/login` with forgot/stubs → Task 7; «Камень» palette and icon theme toggle → Tasks 1, 2, 5; «Мои задачи» list → id → detail → Task 8.
- Calibration stays reachable at `/calibration` until plan 2 moves it into the pack workspace and deletes the sidebar item.
