import { useEffect, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { ADMIN_QUERY_KEYS, ADMIN_REASON_LENGTH, fetchSystemState, updateSystemSettings } from '@/api/server';
import type { SystemState } from '@/api/types';
import NumberInput from '@/components/NumberInput';
import { LIST_POLL_MS } from '@/config';
import { pushToast } from '@/store/useToastStore';
import { formatDateTime, formatMebibytes } from '@/utils/format';

/** Лимит параллельных задач — как проверяет бэкенд. */
const MAX_PARALLEL = 64;

/** Состояние очереди и исполнителей, лимит параллельности и пауза запусков. */
export default function SystemSection() {
  const cache = useQueryClient();
  const system = useQuery({
    queryKey: ADMIN_QUERY_KEYS.system,
    queryFn: fetchSystemState,
    refetchInterval: LIST_POLL_MS,
  });
  const [parallel, setParallel] = useState('1');
  const [paused, setPaused] = useState(false);
  const [reason, setReason] = useState('');

  // Поля формы — текущие настройки сервера (и после применения).
  const configuredParallel = system.data?.configuredParallelism;
  const configuredPaused = system.data?.paused;
  useEffect(() => {
    if (configuredParallel !== undefined && configuredPaused !== undefined) {
      setParallel(String(configuredParallel));
      setPaused(configuredPaused);
    }
  }, [configuredParallel, configuredPaused]);

  const settings = useMutation({
    mutationFn: () => updateSystemSettings({ maxParallel: Number(parallel), paused, reason }),
    onSuccess: () => {
      setReason('');
      void cache.invalidateQueries({ queryKey: ADMIN_QUERY_KEYS.system });
      void cache.invalidateQueries({ queryKey: ADMIN_QUERY_KEYS.audit });
      pushToast('Лимиты обновлены', 'success');
    },
    onError: (e) => pushToast(e.message, 'error'),
  });

  return (
    <section className="card">
      <h2>Система и очередь</h2>
      {system.isError && <div className="error-block">{system.error.message}</div>}
      {system.data && <SystemSummary state={system.data} />}
      <form
        className="form-grid"
        onSubmit={(e) => {
          e.preventDefault();
          settings.mutate();
        }}
      >
        <label className="field">
          Лимит параллельных задач
          <NumberInput min={0} max={MAX_PARALLEL} step={1} required value={parallel} onChange={setParallel} />
        </label>
        <label className="field">
          <input type="checkbox" checked={paused} onChange={(e) => setPaused(e.target.checked)} />
          Пауза новых запусков
        </label>
        <label className="field">
          Причина изменения
          <input
            className="input"
            value={reason}
            onChange={(e) => setReason(e.target.value)}
            required
            minLength={ADMIN_REASON_LENGTH.min}
            maxLength={ADMIN_REASON_LENGTH.max}
          />
        </label>
        <button className="btn" disabled={settings.isPending}>
          Применить
        </button>
      </form>
      <p>Уменьшение лимита не прерывает выполняющиеся задачи. Лимит не может добавить машине памяти.</p>
    </section>
  );
}

function SystemSummary({ state }: { state: SystemState }) {
  return (
    <>
      <div className="kv">
        <div className="kv-item">
          Выполняется <strong>{state.activeJobs}</strong>
        </div>
        <div className="kv-item">
          В очереди <strong>{state.queuedJobs}</strong>
        </div>
        <div className="kv-item">
          Разрешено настройкой <strong>{state.configuredParallelism}</strong>
        </div>
        <div className="kv-item">
          Доступно по ресурсам <strong>{state.effectiveParallelism}</strong>
        </div>
      </div>
      <p>
        Хранилище: {formatMebibytes(state.storageUsedBytes)} / {formatMebibytes(state.storageLimitBytes)}. CPU API:{' '}
        {state.apiAvailableProcessors}, heap: {formatMebibytes(state.apiHeapMaxBytes)}.
      </p>
      {!state.workers.length && (
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
            {state.workers.map((w) => (
              <tr key={w.id}>
                <td>
                  {w.id} ({w.mode})
                </td>
                <td>
                  {w.cpuLimit} / {formatMebibytes(w.memoryLimitBytes)}
                </td>
                <td>{w.capacity}</td>
                <td>{w.activeCount}</td>
                <td>{formatDateTime(w.lastSeenAt)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </>
  );
}
