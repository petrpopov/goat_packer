import { create } from 'zustand';

export type Theme = 'dark' | 'light';
export type Tab = 'pack' | 'schedule';

export interface PackHandoff {
  packJobId: string;
  profileId: string;
}

const THEME_KEY = 'goat-packer-theme';

function readInitialTheme(): Theme {
  try {
    return localStorage.getItem(THEME_KEY) === 'light' ? 'light' : 'dark';
  } catch {
    return 'dark';
  }
}

function applyTheme(theme: Theme): void {
  document.documentElement.dataset.theme = theme;
}

/** Вызывается один раз до рендера, чтобы тема применилась без мигания. */
export function initTheme(): void {
  applyTheme(readInitialTheme());
}

interface UiState {
  theme: Theme;
  activeTab: Tab;
  /** Результат фазы 1, переданный на вкладку расписания («→ В расписание»). */
  packHandoff: PackHandoff | null;
  toggleTheme: () => void;
  setTab: (tab: Tab) => void;
  sendToSchedule: (handoff: PackHandoff) => void;
}

export const useUiStore = create<UiState>((set, get) => ({
  theme: readInitialTheme(),
  activeTab: 'pack',
  packHandoff: null,
  toggleTheme: () => {
    const next: Theme = get().theme === 'dark' ? 'light' : 'dark';
    applyTheme(next);
    try {
      localStorage.setItem(THEME_KEY, next);
    } catch {
      /* приватный режим — тема просто не сохранится */
    }
    set({ theme: next });
  },
  setTab: (tab) => set({ activeTab: tab }),
  sendToSchedule: (handoff) => set({ packHandoff: handoff, activeTab: 'schedule' }),
}));
