import { useQuery } from '@tanstack/react-query';
import { ApiError } from '@/api/http';
import { getJob } from '@/api/services';
import type { JobStatusResponse } from '@/api/types';

const POLL_INTERVAL_MS = 1000;

const ACTIVE_STATUSES = new Set(['QUEUED', 'RUNNING', 'FINALIZING', 'LOST', 'RETRY_WAIT']);

/**
 * Поллинг статуса задачи: GET /api/jobs/{id} раз в 1000 мс,
 * пока статус QUEUED/RUNNING; на терминальном статусе опрос останавливается.
 */
export function useJob(jobId: string | null) {
  return useQuery<JobStatusResponse>({
    queryKey: ['job', jobId],
    enabled: jobId !== null,
    queryFn: ({ signal }) => {
      if (!jobId) throw new Error('Идентификатор задачи не задан');
      // Момент получения — точка отсчёта для локального таймера этапа между опросами
      // (часы клиента и сервера не сравниваются).
      return getJob(jobId, signal).then((job) => ({ ...job, receivedAtMs: Date.now() }));
    },
    retry: (count, error) => !(error instanceof ApiError && error.status >= 400 && error.status < 500) && count < 2,
    refetchInterval: (query) => {
      if (query.state.error instanceof ApiError && [404, 410].includes(query.state.error.status)) return false;
      const status = query.state.data?.status;
      return !status || ACTIVE_STATUSES.has(status) ? POLL_INTERVAL_MS : false;
    },
  });
}
