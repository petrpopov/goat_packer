import { useCallback, useState } from 'react';
import { useMutation, useQuery } from '@tanstack/react-query';
import { ApiError } from '@/api/http';
import { cancelPackCheck, checkPackJob, deleteJob, downloadJobInput } from '@/api/services';
import type { JobArtifact, PackJobInfo } from '@/api/types';
import { useJobView } from '@/hooks/useJob';
import { pushToast } from '@/store/useToastStore';
import { useUiStore } from '@/store/useUiStore';
import { isPlateFile, visibleArtifacts } from '@/utils/artifacts';
import { apiErrorMessage, isActiveStatus, isRunningStatus } from '@/utils/jobs';

/**
 * Открытая задача упаковки (адрес /pack/<id>): статус, проверка нарезкой внутри неё, вход задачи,
 * открытой по ссылке, и действия (проверить, остановить проверку, отменить).
 */
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

  const check = job?.check ?? null;
  const checkActive = isActiveStatus(check?.status);
  const checkRunning = isRunningStatus(check?.status);
  const checkDone = check?.status === 'SUCCEEDED';
  /** Итог для скачивания: после проверки нарезкой — её стол (она могла сдвинуть объекты). */
  const result: PackJobInfo | null = checkDone && check?.pack ? check.pack : (job?.pack ?? null);
  const deliverables: JobArtifact[] = checkDone && check ? check.artifacts : (job?.artifacts ?? []);
  /** Сколько объектов получилось; null — итога ещё нет. */
  const resultCount = job?.status === 'SUCCEEDED' && result ? result.objectCount : null;

  const refetch = () => void query.refetch();
  // Проверка нарезкой идёт внутри той же задачи: адрес не меняется, опрос подхватывает её ход.
  const checkMutation = useMutation({
    mutationFn: checkPackJob,
    onSuccess: refetch,
    onError: (error) => pushToast(apiErrorMessage(error, 'Не удалось запустить проверку'), 'error'),
  });
  const cancelCheckMutation = useMutation({
    mutationFn: cancelPackCheck,
    onSuccess: refetch,
    onError: (error) => pushToast(apiErrorMessage(error, 'Не удалось остановить проверку'), 'error'),
  });
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
    check,
    checkActive,
    checkRunning,
    checkDone,
    result,
    resultCount,
    deliverables,
    plateFile: visibleArtifacts(deliverables).find(isPlateFile),
    startCheck: checkMutation,
    cancelCheck: cancelCheckMutation,
    cancelJob: cancelMutation,
  };
}

export type PackJob = ReturnType<typeof usePackJob>;
