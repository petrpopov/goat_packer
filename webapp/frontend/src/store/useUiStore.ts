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
