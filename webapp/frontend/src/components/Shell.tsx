import type { ReactNode } from 'react';
import { serverMode } from '@/api/http';
import AccountMenu from '@/components/AccountMenu';
import BrandMark from '@/components/BrandMark';
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
];
const JOBS_ITEM: NavItem = { path: '/jobs', label: 'Мои задачи', icon: 'list', match: ['jobs', 'job'] };
const ADMIN_ITEM: NavItem = { path: '/admin', label: 'Администрирование', icon: 'shield', match: ['admin'] };

export default function Shell({ user, children }: { user?: Session; children: ReactNode }) {
  const route = useUiStore((s) => s.route);
  const packJobId = useUiStore((s) => s.packJobId);
  const rail = useUiStore((s) => s.rail);
  const toggleRail = useUiStore((s) => s.toggleRail);
  const theme = useUiStore((s) => s.theme);
  const toggleTheme = useUiStore((s) => s.toggleTheme);
  const onLink = useLinkHandler();

  const navLink = (item: NavItem) => {
    const active = item.match.includes(route.name);
    // «Упаковка» возвращает к открытой задаче, если она есть (бренд — всегда к новой упаковке).
    const href = item.path === '/pack' && packJobId ? `/pack/${packJobId}` : item.path;
    return (
      <a
        key={item.path}
        href={href}
        onClick={onLink}
        data-tip={rail ? item.label : undefined}
        data-tip-pos="right"
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
          data-tip={rail ? 'Развернуть меню' : 'Свернуть меню'}
        >
          <Icon name="sidebar" />
        </button>
        <a className="brand" href="/pack" onClick={onLink}>
          <BrandMark />
          Goat Packer
        </a>
        <div className="topbar-spacer" />
        <button
          type="button"
          className="icon-btn"
          onClick={toggleTheme}
          aria-label="Переключить тему"
          data-tip={theme === 'dark' ? 'Светлая тема' : 'Тёмная тема'}
          data-tip-align="end"
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
          <div
            className="build-tag"
            data-tip-pos="right"
            data-tip={`Версия ${__GOAT_BUILD__.version}, коммит ${__GOAT_BUILD__.commit || 'неизвестен'}`}
          >
            {__GOAT_BUILD__.version}
            {__GOAT_BUILD__.commit && <span> · {__GOAT_BUILD__.commit}</span>}
          </div>
        </nav>
        <main className="app-content">{children}</main>
      </div>
    </div>
  );
}
