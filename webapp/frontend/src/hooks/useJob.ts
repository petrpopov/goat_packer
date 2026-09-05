import { useQuery } from '@tanstack/react-query';
import { getJob } from '@/api/services';
import type { JobStatusResponse } from '@/api/types';

const POLL_INTERVAL_MS = 1000;

const ACTIVE_STATUSES = new Set(['QUEUED', 'RUNNING']);

/**
 * Поллинг статуса задачи: GET /api/jobs/{id} раз в 1000 мс,
 * пока статус QUEUED/RUNNING; на терминальном статусе опрос останавливается.
 */
export function useJob(jobId: string | null) {
  return useQuery<JobStatusResponse>({
    queryKey: ['job', jobId],
    enabled: jobId !== null,
    queryFn: () => {
      if (!jobId) throw new Error('Идентификатор задачи не задан');
      return getJob(jobId);
    },
    refetchInterval: (query) => {
      const status = query.state.data?.status;
      return status && ACTIVE_STATUSES.has(status) ? POLL_INTERVAL_MS : false;
    },
  });
}
