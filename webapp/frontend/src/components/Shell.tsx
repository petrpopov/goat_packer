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
