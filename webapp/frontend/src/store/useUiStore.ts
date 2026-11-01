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
  window.addEventListener('popstate', () => useUiStore.setState(routeState(location.pathname)));
}

/**
 * Маршрут и задача упаковки: страница упаковки смонтирована и на других разделах, поэтому открытая
 * задача (/pack/<id>) переживает уход в расписание; пункт меню «Упаковка» ведёт обратно к ней.
 */
function routeState(path: string): Pick<UiState, 'route'> & Partial<Pick<UiState, 'packJobId'>> {
  const route = parseRoute(path);
  return route.name === 'pack' ? { route, packJobId: route.jobId ?? null } : { route };
}

interface UiState {
  theme: Theme;
  route: Route;
  /** Задача, открытая в упаковке (/pack/<id>); null — черновик новой упаковки. */
  packJobId: string | null;
  /** Сайдбар свёрнут в полосу иконок. */
  rail: boolean;
  /** Результат упаковки, переданный в расписание («В расписание»). */
  packHandoff: PackHandoff | null;
  /** Открыта боковая панель калибровки смены прутка в упаковке. */
  calibOpen: boolean;
  toggleTheme: () => void;
  toggleRail: () => void;
  navigate: (path: string, options?: { replace?: boolean }) => void;
  sendToSchedule: (handoff: PackHandoff) => void;
  setCalibOpen: (open: boolean) => void;
}

export const useUiStore = create<UiState>((set, get) => ({
  theme: readInitialTheme(),
  packJobId: null,
  ...routeState(typeof location === 'undefined' ? '/' : location.pathname),
  rail: readStored(RAIL_KEY) === '1',
  packHandoff: null,
  calibOpen: false,
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
    set(routeState(path));
    window.scrollTo(0, 0);
  },
  sendToSchedule: (handoff) => {
    set({ packHandoff: handoff });
    get().navigate('/schedule');
  },
  setCalibOpen: (open) => set({ calibOpen: open }),
}));
