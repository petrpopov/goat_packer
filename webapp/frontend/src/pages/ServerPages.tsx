import { useEffect, useState } from 'react';
import type { ReactNode, FormEvent } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { ApiError, serverApi } from '@/api/http';
import { pushToast } from '@/store/useToastStore';
import { useUiStore } from '@/store/useUiStore';
import { useJob } from '@/hooks/useJob';
import AmsCalculator from '@/components/AmsCalculator';
import JobCard from '@/components/JobCard';
import { calibrationValues } from '@/utils/calibration';
import StatusPill from '@/components/StatusPill';
import type { JobStatusResponse } from '@/api/types';

export interface Session {
  id: string;
  kind: 'GUEST' | 'USER';
  login: string | null;
  role: 'USER' | 'ADMIN';
  storageQuotaBytes: number;
}
async function loadSession(): Promise<Session> {
  await serverApi.get('/auth/csrf');
  let user: Session;
  try {
    user = await serverApi.get<Session>('/auth/me');
  } catch (error) {
    if (!(error instanceof ApiError) || error.status !== 401) throw error;
    user = await serverApi.post<Session>('/auth/guest');
  }
  const jobId = /^\/jobs\/([a-z0-9-]+)$/.exec(location.pathname)?.[1];
  const access = new URLSearchParams(location.hash.slice(1)).get('access');
  if (jobId && access) {
    await serverApi.post('/access', { jobId, token: access });
    history.replaceState(null, '', location.pathname + location.search);
  }
  return user;
}
export function useSession() {
  return useQuery({ queryKey: ['session'], queryFn: loadSession, retry: false, staleTime: 60000 });
}
export function ServerGate({ children }: { children: ReactNode }) {
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
export function AccountPage() {
  const session = useSession();
  const [mode, setMode] = useState<'login' | 'register' | 'reset'>('login');
  const [login, setLogin] = useState('');
  const [password, setPassword] = useState('');
  const [token, setToken] = useState('');
  const mutation = useMutation({
    mutationFn: async () => {
      await serverApi.post(`/auth/${mode}`, mode === 'reset' ? { token, password } : { login, password });
      setPassword('');
      location.reload();
    },
    onError: (e) => pushToast(e.message, 'error'),
  });
  const submit = (e: FormEvent) => {
    e.preventDefault();
    mutation.mutate();
  };
  return (
    <div className="page">
      <h1 className="page-title">Аккаунт</h1>
      <section className="card">
        <p>
          {session.data?.kind === 'GUEST'
            ? 'Гостевой доступ: результаты хранятся 24 часа после завершения. Регистрация сохранит ваши текущие задачи.'
            : `Вы вошли как ${session.data?.login}`}
        </p>
        {session.data?.kind === 'USER' ? (
          <button className="btn" onClick={() => void serverApi.post('/auth/logout').then(() => location.reload())}>
            Выйти
          </button>
        ) : (
          <form className="stack" onSubmit={submit}>
            <div className="form-actions">
              {(['login', 'register', 'reset'] as const).map((m) => (
                <button
                  type="button"
                  className={`btn ${mode === m ? 'btn-primary' : ''}`}
                  key={m}
                  onClick={() => setMode(m)}
                >
                  {m === 'login' ? 'Войти' : m === 'register' ? 'Регистрация' : 'Сброс пароля'}
                </button>
              ))}
            </div>
            {mode === 'reset' ? (
              <label className="field">
                Одноразовый токен от администратора
                <input
                  className="input"
                  value={token}
                  onChange={(e) => setToken(e.target.value)}
                  required
                  autoComplete="off"
                />
              </label>
            ) : (
              <label className="field">
                Имя пользователя
                <input
                  className="input"
                  value={login}
                  onChange={(e) => setLogin(e.target.value)}
                  required
                  minLength={3}
                  maxLength={120}
                  autoComplete="username"
                />
              </label>
            )}
            <label className="field">
              Пароль (минимум 12 символов)
              <input
                className="input"
                type="password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                required
                minLength={12}
                maxLength={72}
                autoComplete={mode === 'login' ? 'current-password' : 'new-password'}
              />
            </label>
            <button className="btn btn-primary" disabled={mutation.isPending}>
              Продолжить
            </button>
          </form>
        )}
      </section>
    </div>
  );
}
export function HistoryPage() {
  const [page, setPage] = useState(0);
  const [selected, setSelected] = useState<string | null>(
    () => /^\/jobs\/([a-z0-9-]+)$/.exec(location.pathname)?.[1] ?? null
  );
  const list = useQuery({
    queryKey: ['history', page],
    queryFn: () => serverApi.get<{ items: JobStatusResponse[]; total: number }>(`/jobs?page=${page}`),
    refetchInterval: 5000,
  });
  const uploads = useQuery({
    queryKey: ['my-uploads'],
    queryFn: () =>
      serverApi.get<{ fileId: string; originalName: string; sizeBytes: number; state: string }[]>('/uploads'),
    refetchInterval: 10000,
  });
  const removeUpload = useMutation({
    mutationFn: (id: string) => serverApi.delete(`/uploads/${id}`),
    onSuccess: () => void uploads.refetch(),
    onError: (e) => pushToast(e.message, 'error'),
  });
  const current = useJob(selected);
  const remove = useMutation({
    mutationFn: (id: string) => serverApi.delete(`/jobs/${id}`),
    onSuccess: () => {
      setSelected(null);
      void list.refetch();
    },
    onError: (e) => pushToast(e.message, 'error'),
  });
  const setTab = useUiStore((s) => s.setTab);
  const send = useUiStore((s) => s.sendToSchedule);
  return (
    <div className="page">
      <h1 className="page-title">Мои задачи</h1>
      {list.isError && <div className="error-block">{list.error.message}</div>}
      <section className="card">
        <div className="table-wrap">
          <table className="table">
            <thead>
              <tr>
                <th>Файл / задача</th>
                <th>Тип</th>
                <th>Статус</th>
                <th>Создана</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {list.data?.items.map((j) => (
                <tr key={j.jobId}>
                  <td>{j.originalName ?? j.jobId}</td>
                  <td>{j.kind}</td>
                  <td>
                    <StatusPill status={j.status} />
                  </td>
                  <td>{new Date(j.createdAt).toLocaleString()}</td>
                  <td>
                    <button className="btn" onClick={() => setSelected(j.jobId)}>
                      Открыть
                    </button>
                    {!['QUEUED', 'RUNNING', 'FINALIZING', 'LOST', 'RETRY_WAIT'].includes(j.status) && (
                      <button
                        className="btn"
                        disabled={remove.isPending}
                        onClick={() => {
                          if (window.confirm('Удалить задачу и ссылки на её результаты?')) remove.mutate(j.jobId);
                        }}
                      >
                        Удалить
                      </button>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        {!list.data?.items.length && (
          <p>
            Пока нет задач.{' '}
            <button className="btn" onClick={() => setTab('pack')}>
              Упаковать файл
            </button>
          </p>
        )}
        <div className="form-actions">
          <button className="btn" disabled={page === 0} onClick={() => setPage((p) => p - 1)}>
            Назад
          </button>
          <span>{page + 1}</span>
          <button
            className="btn"
            disabled={(page + 1) * 20 >= (list.data?.total ?? 0)}
            onClick={() => setPage((p) => p + 1)}
          >
            Далее
          </button>
        </div>
      </section>
      <section className="card">
        <h2>Загруженные исходники</h2>
        <p>Удаление исходника не отменяет уже принятые задачи. Общий blob освободится, когда исчезнут все ссылки.</p>
        {uploads.data?.map((file) => (
          <div className="artifact-row" key={file.fileId}>
            <span>
              {file.originalName} · {mib(file.sizeBytes)} · {file.state}
            </span>
            <button
              className="btn"
              disabled={removeUpload.isPending || file.state === 'RECEIVING'}
              onClick={() => {
                if (window.confirm('Удалить этот исходник из профиля?')) removeUpload.mutate(file.fileId);
              }}
            >
              Удалить
            </button>
          </div>
        ))}
      </section>
      <JobCard
        jobId={selected}
        job={current.data ?? null}
        pollError={current.error}
        onDismiss={() => setSelected(null)}
        successContent={
          current.data?.pack ? (
            <button
              className="btn"
              onClick={() => send({ packJobId: current.data!.jobId, profileId: current.data!.pack!.profileId })}
            >
              → В расписание
            </button>
          ) : null
        }
      />
      {current.data?.status === 'SUCCEEDED' && current.data.calibration ? (
        <AmsCalculator
          key={current.data.jobId}
          profileId={current.data.calibration.profileId}
          initial={calibrationValues(current.data.calibration)}
        />
      ) : null}
    </div>
  );
}
interface User extends Session {
  blocked: boolean;
  blockReason: string | null;
  createdAt: string;
  lastSeenAt: string;
  storageUsedBytes: number;
  resetRequired: boolean;
}
interface Worker {
  id: string;
  cpuLimit: number;
  memoryLimitBytes: number;
  capacity: number;
  activeCount: number;
  lastSeenAt: string;
  mode: string;
}
interface SystemState {
  configuredParallelism: number;
  effectiveParallelism: number;
  paused: boolean;
  activeJobs: number;
  queuedJobs: number;
  workers: Worker[];
  storageUsedBytes: number;
  storageLimitBytes: number;
  apiAvailableProcessors: number;
  apiHeapMaxBytes: number;
}
interface Audit {
  id: string;
  action: string;
  targetId: string | null;
  details: string;
  createdAt: string;
}
const mib = (v: number) => `${(v / 1024 / 1024).toFixed(1)} МиБ`;
export function AdminPage() {
  const session = useSession();
  const cache = useQueryClient();
  const [kind, setKind] = useState('');
  const [query, setQuery] = useState('');
  const [page, setPage] = useState(0);
  const [target, setTarget] = useState<User | null>(null);
  const [action, setAction] = useState<'block' | 'reset'>('block');
  const [reason, setReason] = useState('');
  const [resetToken, setResetToken] = useState('');
  const [parallel, setParallel] = useState('1');
  const [paused, setPaused] = useState(false);
  const [configReason, setConfigReason] = useState('');
  const enabled = session.data?.role === 'ADMIN';
  const system = useQuery({
    queryKey: ['admin-system'],
    queryFn: () => serverApi.get<SystemState>('/admin/system'),
    enabled,
    refetchInterval: 5000,
  });
  const users = useQuery({
    queryKey: ['admin-users', kind, query, page],
    queryFn: () =>
      serverApi.get<{ items: User[]; total: number }>(
        `/admin/users?kind=${kind}&query=${encodeURIComponent(query)}&page=${page}`
      ),
    enabled,
  });
  const audit = useQuery({ queryKey: ['admin-audit'], queryFn: () => serverApi.get<Audit[]>('/admin/audit'), enabled });
  const configuredParallel = system.data?.configuredParallelism;
  const configuredPaused = system.data?.paused;
  useEffect(() => {
    if (configuredParallel !== undefined && configuredPaused !== undefined) {
      setParallel(String(configuredParallel));
      setPaused(configuredPaused);
    }
  }, [configuredParallel, configuredPaused]);
  const mutate = useMutation({
    mutationFn: async () => {
      if (!target) return;
      if (action === 'reset') {
        const r = await serverApi.post<{ token: string }>(`/admin/users/${target.id}/password-reset`, { reason });
        setResetToken(r.token);
      } else await serverApi.post(`/admin/users/${target.id}/block`, { blocked: !target.blocked, reason });
    },
    onSuccess: () => {
      setTarget(null);
      setReason('');
      void cache.invalidateQueries({ queryKey: ['admin-users'] });
      void cache.invalidateQueries({ queryKey: ['admin-audit'] });
    },
    onError: (e) => pushToast(e.message, 'error'),
  });
  const settings = useMutation({
    mutationFn: () => serverApi.post('/admin/system', { maxParallel: Number(parallel), paused, reason: configReason }),
    onSuccess: () => {
      setConfigReason('');
      void cache.invalidateQueries({ queryKey: ['admin-system'] });
      void cache.invalidateQueries({ queryKey: ['admin-audit'] });
      pushToast('Лимиты обновлены', 'success');
    },
    onError: (e) => pushToast(e.message, 'error'),
  });
  if (!enabled) return <div className="card error-block">Доступ только администратору</div>;
  return (
    <div className="page">
      <h1 className="page-title">Администрирование</h1>
      {[system.error, users.error, audit.error].filter(Boolean).map((e, i) => (
        <div className="error-block" key={i}>
          {e?.message}
        </div>
      ))}
      <section className="card">
        <h2>Система и очередь</h2>
        {system.data && (
          <>
            <div className="kv">
              <div className="kv-item">
                Выполняется <strong>{system.data.activeJobs}</strong>
              </div>
              <div className="kv-item">
                В очереди <strong>{system.data.queuedJobs}</strong>
              </div>
              <div className="kv-item">
                Разрешено настройкой <strong>{system.data.configuredParallelism}</strong>
              </div>
              <div className="kv-item">
                Доступно по ресурсам <strong>{system.data.effectiveParallelism}</strong>
              </div>
            </div>
            <p>
              Хранилище: {mib(system.data.storageUsedBytes)} / {mib(system.data.storageLimitBytes)}. CPU API:{' '}
              {system.data.apiAvailableProcessors}, heap: {mib(system.data.apiHeapMaxBytes)}.
            </p>
            {!system.data.workers.length && (
              <div className="error-block">Нет зарегистрированных исполнителей. Задачи будут ждать в БД.</div>
            )}
            <div className="table-wrap">
              <table className="table">
                <thead>
                  <tr>
                    <th>Исполнитель</th>
                    <th>CPU / RAM бюджета</th>
                    <th>Слотов</th>
                    <th>Занято</th>
                    <th>Heartbeat</th>
                  </tr>
                </thead>
                <tbody>
                  {system.data.workers.map((w) => (
                    <tr key={w.id}>
                      <td>
                        {w.id} ({w.mode})
                      </td>
                      <td>
                        {w.cpuLimit} / {mib(w.memoryLimitBytes)}
                      </td>
                      <td>{w.capacity}</td>
                      <td>{w.activeCount}</td>
                      <td>{new Date(w.lastSeenAt).toLocaleString()}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </>
        )}
        <form
          className="form-grid"
          onSubmit={(e) => {
            e.preventDefault();
            settings.mutate();
          }}
        >
          <label className="field">
            Лимит параллельных задач
            <input
              className="input"
              type="number"
              min={0}
              max={64}
              step={1}
              required
              value={parallel}
              onChange={(e) => setParallel(e.target.value)}
            />
          </label>
          <label className="field">
            <input type="checkbox" checked={paused} onChange={(e) => setPaused(e.target.checked)} />
            Пауза новых запусков
          </label>
          <label className="field">
            Причина изменения
            <input
              className="input"
              value={configReason}
              onChange={(e) => setConfigReason(e.target.value)}
              required
              minLength={3}
              maxLength={500}
            />
          </label>
          <button className="btn" disabled={settings.isPending}>
            Применить
          </button>
        </form>
        <p>Уменьшение лимита не прерывает выполняющиеся задачи. Лимит не может добавить машине памяти.</p>
      </section>
      <section className="card">
        <h2>Пользователи и гости</h2>
        <div className="form-grid">
          <label className="field">
            Тип
            <select
              className="input"
              value={kind}
              onChange={(e) => {
                setKind(e.target.value);
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
              maxLength={100}
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
                    {mib(u.storageUsedBytes)} / {mib(u.storageQuotaBytes)}
                  </td>
                  <td>
                    {u.role !== 'ADMIN' && (
                      <>
                        <button
                          className="btn"
                          onClick={() => {
                            setTarget(u);
                            setAction('block');
                            setResetToken('');
                          }}
                        >
                          {u.blocked ? 'Разблокировать' : 'Заблокировать'}
                        </button>
                        {u.kind === 'USER' && (
                          <button
                            className="btn"
                            onClick={() => {
                              setTarget(u);
                              setAction('reset');
                              setResetToken('');
                            }}
                          >
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
        <div className="form-actions">
          <button className="btn" disabled={!page} onClick={() => setPage((p) => p - 1)}>
            Назад
          </button>
          <span>
            {page + 1} · всего {users.data?.total ?? 0}
          </span>
          <button
            className="btn"
            disabled={(page + 1) * 30 >= (users.data?.total ?? 0)}
            onClick={() => setPage((p) => p + 1)}
          >
            Далее
          </button>
        </div>
        <p>
          Блокировка гостя действует на его идентификатор. Очистку cookies и новый IP она не предотвращает —
          дополнительно работают глобальные квоты.
        </p>
        {target && (
          <form
            className="panel stack"
            onSubmit={(e) => {
              e.preventDefault();
              mutate.mutate();
            }}
          >
            <strong>
              {action === 'reset'
                ? 'Сбросить пароль и отозвать все сессии'
                : target.blocked
                  ? 'Разрешить доступ'
                  : 'Заблокировать доступ и отменить активные задачи'}
              : {target.login ?? target.id}?
            </strong>
            <input
              aria-label="Причина действия"
              className="input"
              required
              minLength={3}
              maxLength={500}
              value={reason}
              onChange={(e) => setReason(e.target.value)}
            />
            <div className="form-actions">
              <button className="btn btn-primary" disabled={mutate.isPending}>
                Подтвердить
              </button>
              <button type="button" className="btn" onClick={() => setTarget(null)}>
                Отмена
              </button>
            </div>
          </form>
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
      <section className="card">
        <h2>Журнал административных действий</h2>
        {audit.data?.map((a) => (
          <p key={a.id}>
            <span className="mono">
              {new Date(a.createdAt).toLocaleString()} {a.action}
            </span>
            <br />
            {a.targetId} · {a.details}
          </p>
        ))}
      </section>
    </div>
  );
}
