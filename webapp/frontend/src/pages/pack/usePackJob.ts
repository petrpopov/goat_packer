import { useCallback, useState } from 'react';
import { useMutation, useQuery } from '@tanstack/react-query';
import { ApiError } from '@/api/http';
import { deleteJob, downloadJobInput } from '@/api/services';
import type { JobArtifact, PackJobInfo } from '@/api/types';
import { useJobView } from '@/hooks/useJob';
import { pushToast } from '@/store/useToastStore';
import { useUiStore } from '@/store/useUiStore';
import { isPlateFile, visibleArtifacts } from '@/utils/artifacts';
import { isRunningStatus } from '@/utils/jobs';

/** Открытая задача упаковки (адрес /pack/<id>): статус, вход задачи, открытой по ссылке, и отмена. */
export function usePackJob(jobId: string | null) {
  const navigate = useUiStore((state) => state.navigate);
  const { query, job, gone, active } = useJobView(jobId);
  /** Задачи, запущенные с этой вкладки: их модель — уже разобранный файл формы, качать вход не нужно. */
  const [startedHere, setStartedHere] = useState<ReadonlySet<string>>(() => new Set());
  const markStartedHere = useCallback((id: string) => setStartedHere((ids) => new Set(ids).add(id)), []);

  /** Открыта задача (её параметры — только для чтения, «Упаковать заново» возвращает к черновику). */
  const viewing = jobId !== null;
  /** Задача открыта по ссылке или из списка: модель и параметры — с сервера, не из формы. */
  const external = viewing && !startedHere.has(jobId);
  const originalName = job?.kind === 'pack' ? job.originalName : undefined;
  const input = useQuery({
    queryKey: ['job-input', jobId],
    enabled: external && Boolean(originalName),
    queryFn: ({ signal }) => downloadJobInput(jobId!, originalName!, signal),
    staleTime: Infinity,
    retry: false,
  });
  const inputFile = external ? (input.data ?? null) : null;

  const result: PackJobInfo | null = job?.pack ?? null;
  const deliverables: JobArtifact[] = job?.artifacts ?? [];
  /** Сколько объектов получилось; null — итога ещё нет. */
  const resultCount = job?.status === 'SUCCEEDED' && result ? result.objectCount : null;
  /**
   * Сколько объектов на столе — итог, а пока его нет, число последней раскладки пакера (layout-событие):
   * раскладка видна раньше, чем стол собран. preliminary — число ещё может измениться (следующий уровень
   * растра пакера, а у окна печати — его нарезки).
   */
  const objectCount = resultCount ?? job?.layout?.objects ?? null;
  const countPreliminary = resultCount == null && objectCount != null;

  const cancelMutation = useMutation({
    mutationFn: (id: string) => deleteJob(id),
    onSuccess: () => navigate('/pack'),
    onError: (error) => {
      if (error instanceof ApiError && error.status === 404) navigate('/pack');
      else pushToast(error.message, 'error');
    },
  });

  return {
    query,
    job,
    gone,
    active,
    viewing,
    external,
    markStartedHere,
    input,
    inputFile,
    running: isRunningStatus(job?.status),
    result,
    resultCount,
    objectCount,
    countPreliminary,
    deliverables,
    plateFile: visibleArtifacts(deliverables).find(isPlateFile),
    cancelJob: cancelMutation,
  };
}

export type PackJob = ReturnType<typeof usePackJob>;
