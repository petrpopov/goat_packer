import { useQuery } from '@tanstack/react-query';
import { ADMIN_QUERY_KEYS, fetchAudit } from '@/api/server';
import { useSession } from '@/hooks/useSession';
import { formatDateTime } from '@/utils/format';
import SystemSection from './SystemSection';
import UsersSection from './UsersSection';

/** Администрирование (только ADMIN): система и очередь, пользователи, журнал действий. */
export default function AdminPage() {
  const session = useSession();
  if (session.data?.role !== 'ADMIN') return <div className="card error-block">Доступ только администратору</div>;
  return (
    <div className="page">
      <h1 className="page-title">Администрирование</h1>
      <SystemSection />
      <UsersSection />
      <AuditSection />
    </div>
  );
}

function AuditSection() {
  const audit = useQuery({ queryKey: ADMIN_QUERY_KEYS.audit, queryFn: fetchAudit });
  return (
    <section className="card">
      <h2>Журнал административных действий</h2>
      {audit.isError && <div className="error-block">{audit.error.message}</div>}
      {audit.data?.map((entry) => (
        <p key={entry.id}>
          <span className="mono">
            {formatDateTime(entry.createdAt)} {entry.action}
          </span>
          <br />
          {entry.targetId} · {entry.details}
        </p>
      ))}
    </section>
  );
}
