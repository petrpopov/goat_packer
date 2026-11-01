import { useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { ADMIN_QUERY_KEYS, ADMIN_REASON_LENGTH, listUsers, resetUserPassword, setUserBlocked } from '@/api/server';
import type { AdminUser, UserKindFilter } from '@/api/types';
import Pager from '@/components/Pager';
import { ADMIN_USERS_PAGE_SIZE } from '@/config';
import { pushToast } from '@/store/useToastStore';
import { formatMebibytes } from '@/utils/format';

const QUERY_MAX_LENGTH = 100;

type UserAction = 'block' | 'reset';

/** Пользователи и гости: поиск, блокировка, сброс пароля одноразовым токеном. */
export default function UsersSection() {
  const cache = useQueryClient();
  const [kind, setKind] = useState<UserKindFilter>('');
  const [query, setQuery] = useState('');
  const [page, setPage] = useState(0);
  const [target, setTarget] = useState<{ user: AdminUser; action: UserAction } | null>(null);
  const [resetToken, setResetToken] = useState('');
  const users = useQuery({
    queryKey: [...ADMIN_QUERY_KEYS.users, kind, query, page],
    queryFn: () => listUsers(kind, query, page),
  });
  const total = users.data?.total ?? 0;

  const choose = (user: AdminUser, action: UserAction) => {
    setTarget({ user, action });
    setResetToken('');
  };

  return (
    <section className="card">
      <h2>Пользователи и гости</h2>
      {users.isError && <div className="error-block">{users.error.message}</div>}
      <div className="form-grid">
        <label className="field">
          Тип
          <select
            className="input"
            value={kind}
            onChange={(e) => {
              setKind(e.target.value as UserKindFilter);
              setPage(0);
            }}
          >
            <option value="">Все</option>
            <option value="USER">Аккаунты</option>
            <option value="GUEST">Гости</option>
          </select>
        </label>
        <label className="field">
          Имя или ID
          <input
            className="input"
            value={query}
            onChange={(e) => {
              setQuery(e.target.value);
              setPage(0);
            }}
            maxLength={QUERY_MAX_LENGTH}
          />
        </label>
      </div>
      <div className="table-wrap">
        <table className="table">
          <thead>
            <tr>
              <th>Имя / ID</th>
              <th>Тип</th>
              <th>Доступ</th>
              <th>Хранилище</th>
              <th>Действия</th>
            </tr>
          </thead>
          <tbody>
            {users.data?.items.map((u) => (
              <tr key={u.id}>
                <td>
                  {u.login ?? 'Гость'}
                  <br />
                  <small className="mono">{u.id}</small>
                </td>
                <td>
                  {u.kind} / {u.role}
                </td>
                <td>
                  {u.blocked ? 'Заблокирован' : 'Разрешён'}
                  {u.resetRequired && ' · нужен сброс'}
                  <br />
                  {u.blockReason}
                </td>
                <td>
                  {formatMebibytes(u.storageUsedBytes)} / {formatMebibytes(u.storageQuotaBytes)}
                </td>
                <td>
                  {u.role !== 'ADMIN' && (
                    <>
                      <button className="btn" onClick={() => choose(u, 'block')}>
                        {u.blocked ? 'Разблокировать' : 'Заблокировать'}
                      </button>
                      {u.kind === 'USER' && (
                        <button className="btn" onClick={() => choose(u, 'reset')}>
                          Сбросить пароль
                        </button>
                      )}
                    </>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <Pager
        page={page}
        total={total}
        pageSize={ADMIN_USERS_PAGE_SIZE}
        onChange={setPage}
        label={`${page + 1} · всего ${total}`}
      />
      <p>
        Блокировка гостя действует на его идентификатор. Очистку cookies и новый IP она не предотвращает — дополнительно
        работают глобальные квоты.
      </p>
      {target && (
        <UserActionForm
          {...target}
          onDone={(token) => {
            setTarget(null);
            if (token) setResetToken(token);
            void cache.invalidateQueries({ queryKey: ADMIN_QUERY_KEYS.users });
            void cache.invalidateQueries({ queryKey: ADMIN_QUERY_KEYS.audit });
          }}
          onCancel={() => setTarget(null)}
        />
      )}
      {resetToken && (
        <div className="panel stack">
          <strong>Одноразовый токен на 15 минут. Передайте пользователю безопасным способом.</strong>
          <input className="input mono" readOnly value={resetToken} />
          <p>Старые сессии отозваны. Пользователь задаёт новый пароль в разделе «Аккаунт → Сброс пароля».</p>
          <button className="btn" onClick={() => setResetToken('')}>
            Скрыть токен
          </button>
        </div>
      )}
    </section>
  );
}

/** Подтверждение действия с причиной; onDone получает токен сброса пароля (у блокировки — null). */
function UserActionForm({
  user,
  action,
  onDone,
  onCancel,
}: {
  user: AdminUser;
  action: UserAction;
  onDone: (resetToken: string | null) => void;
  onCancel: () => void;
}) {
  const [reason, setReason] = useState('');
  const mutation = useMutation({
    mutationFn: async () =>
      action === 'reset'
        ? (await resetUserPassword(user.id, reason)).token
        : (await setUserBlocked(user.id, !user.blocked, reason), null),
    onSuccess: onDone,
    onError: (e) => pushToast(e.message, 'error'),
  });
  const question =
    action === 'reset'
      ? 'Сбросить пароль и отозвать все сессии'
      : user.blocked
        ? 'Разрешить доступ'
        : 'Заблокировать доступ и отменить активные задачи';
  return (
    <form
      className="panel stack"
      onSubmit={(e) => {
        e.preventDefault();
        mutation.mutate();
      }}
    >
      <strong>
        {question}: {user.login ?? user.id}?
      </strong>
      <input
        aria-label="Причина действия"
        className="input"
        required
        minLength={ADMIN_REASON_LENGTH.min}
        maxLength={ADMIN_REASON_LENGTH.max}
        value={reason}
        onChange={(e) => setReason(e.target.value)}
      />
      <div className="form-actions">
        <button className="btn btn-primary" disabled={mutation.isPending}>
          Подтвердить
        </button>
        <button type="button" className="btn" onClick={onCancel}>
          Отмена
        </button>
      </div>
    </form>
  );
}
