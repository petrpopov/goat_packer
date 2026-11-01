import { useQuery } from '@tanstack/react-query';
import { ApiError } from '@/api/http';
import { getJob } from '@/api/services';
import type { JobStatusResponse } from '@/api/types';
import { JOB_POLL_MS } from '@/config';
import { isActiveStatus, isJobGone } from '@/utils/jobs';

/**
 * Поллинг статуса задачи: GET /api/jobs/{id} раз в JOB_POLL_MS, пока идёт она сама или её проверка
 * нарезкой (check — внутри той же задачи); на терминальном статусе обеих опрос останавливается.
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
      if (isJobGone(query.state.error)) return false;
      const job = query.state.data;
      return !job || isActiveStatus(job.status) || isActiveStatus(job.check?.status) ? JOB_POLL_MS : false;
    },
  });
}

/** Задача и её состояние для страниц: gone — её больше нет, active — ещё идёт (или статус не получен). */
export function useJobView(jobId: string | null) {
  const query = useJob(jobId);
  const job = query.data ?? null;
  const gone = isJobGone(query.error);
  const active = jobId !== null && !gone && (!job || isActiveStatus(job.status));
  return { query, job, gone, active };
}
