import type { ReactNode } from 'react';
import { useSession } from '@/hooks/useSession';

/** Серверный режим: приложение — только после установленной сессии (иначе запросы получат 401/403). */
export default function ServerGate({ children }: { children: ReactNode }) {
  const session = useSession();
  if (session.isPending) return <div className="card">Подключение к серверу…</div>;
  if (session.isError)
    return (
      <div className="card error-block">
        {session.error.message}
        <button className="btn" onClick={() => void session.refetch()}>
          Повторить
        </button>
      </div>
    );
  return <>{children}</>;
}
