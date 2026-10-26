import { serverMode } from '@/api/http';
import { ServerGate, useSession, AccountPage, HistoryPage, AdminPage } from '@/pages/ServerPages';
import type { Session } from '@/pages/ServerPages';
import ToastHost from '@/components/ToastHost';
import CalibrationPage from '@/pages/CalibrationPage';
import PackPage from '@/pages/PackPage';
import SchedulePage from '@/pages/SchedulePage';
import { useUiStore } from '@/store/useUiStore';

export default function App() {
  return serverMode ? (
    <ServerGate>
      <ServerApplication />
    </ServerGate>
  ) : (
    <Shell />
  );
}
function ServerApplication() {
  const session = useSession();
  return <Shell user={session.data} />;
}
function Shell({ user }: { user?: Session }) {
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
        {serverMode && (
          <button className="btn-ghost" onClick={() => setTab('account')}>
            {user?.login ?? 'Гостевой доступ'}
          </button>
        )}
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
        <button
          type="button"
          className={`tab${activeTab === 'calibration' ? ' active' : ''}`}
          onClick={() => setTab('calibration')}
        >
          Калибровка AMS
        </button>
        {serverMode && (
          <>
            <button
              type="button"
              className={`tab${activeTab === 'history' ? ' active' : ''}`}
              onClick={() => setTab('history')}
            >
              Мои задачи
            </button>
            <button
              type="button"
              className={`tab${activeTab === 'account' ? ' active' : ''}`}
              onClick={() => setTab('account')}
            >
              Аккаунт
            </button>
          </>
        )}
        {user?.role === 'ADMIN' && (
          <button
            type="button"
            className={`tab${activeTab === 'admin' ? ' active' : ''}`}
            onClick={() => setTab('admin')}
          >
            Админка
          </button>
        )}
      </nav>

      <main className="app-main">
        <div hidden={activeTab !== 'pack'}>
          <PackPage />
        </div>
        <div hidden={activeTab !== 'schedule'}>
          <SchedulePage />
        </div>
        <div hidden={activeTab !== 'calibration'}>
          <CalibrationPage />
        </div>
        {serverMode && (
          <>
            <div hidden={activeTab !== 'history'}>
              <HistoryPage />
            </div>
            <div hidden={activeTab !== 'account'}>
              <AccountPage />
            </div>
          </>
        )}
        {user?.role === 'ADMIN' && (
          <div hidden={activeTab !== 'admin'}>
            <AdminPage />
          </div>
        )}
      </main>

      <ToastHost />
    </div>
  );
}
