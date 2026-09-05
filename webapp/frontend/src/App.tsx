import ToastHost from '@/components/ToastHost';
import PackPage from '@/pages/PackPage';
import SchedulePage from '@/pages/SchedulePage';
import { useUiStore } from '@/store/useUiStore';

export default function App() {
  const activeTab = useUiStore((state) => state.activeTab);
  const setTab = useUiStore((state) => state.setTab);
  const theme = useUiStore((state) => state.theme);
  const toggleTheme = useUiStore((state) => state.toggleTheme);

  return (
    <div className="app-shell">
      <header className="app-header">
        <div className="app-header-brand">
          <span className="brand-dot" aria-hidden="true">
            GP
          </span>
          Goat Packer · упаковка и расписание
        </div>
        <div className="topbar-spacer" />
        <button type="button" className="btn-ghost" onClick={toggleTheme}>
          {theme === 'dark' ? 'Светлая тема' : 'Тёмная тема'}
        </button>
      </header>

      <nav className="tabs" aria-label="Разделы">
        <button type="button" className={`tab${activeTab === 'pack' ? ' active' : ''}`} onClick={() => setTab('pack')}>
          Фаза 1 · Упаковка
        </button>
        <button
          type="button"
          className={`tab${activeTab === 'schedule' ? ' active' : ''}`}
          onClick={() => setTab('schedule')}
        >
          Фаза 2 · Расписание
        </button>
      </nav>

      <main className="app-main">{activeTab === 'pack' ? <PackPage /> : <SchedulePage />}</main>

      <ToastHost />
    </div>
  );
}
