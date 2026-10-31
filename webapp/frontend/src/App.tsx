import { useEffect } from 'react';
import { serverMode } from '@/api/http';
import { AdminPage, HistoryPage, ServerGate, useSession } from '@/pages/ServerPages';
import type { Session } from '@/pages/ServerPages';
import Shell from '@/components/Shell';
import ToastHost from '@/components/ToastHost';
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
  const navigate = useUiStore((s) => s.navigate);
  const setCalibOpen = useUiStore((s) => s.setCalibOpen);
  useEffect(() => {
    // Старые ссылки /calibration открывают панель калибровки в упаковке.
    if (route.name === 'calibration') {
      setCalibOpen(true);
      navigate('/pack', { replace: true });
    }
  }, [route.name, navigate, setCalibOpen]);
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
      {serverMode && (visible === 'jobs' || visible === 'job') && <HistoryPage />}
      {serverMode && route.name === 'login' && <LoginPage step={route.step} />}
      {serverMode && visible === 'admin' && <AdminPage />}
      <ToastHost />
    </Shell>
  );
}
