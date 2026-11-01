import { useMutation } from '@tanstack/react-query';
import type { ReactNode } from 'react';
import { ApiError, serverMode } from '@/api/http';
import { deleteJob } from '@/api/services';
import type { CalibrationInfo, JobStatusResponse, PackJobInfo } from '@/api/types';
import { pushToast } from '@/store/useToastStore';
import { formatDateTime, formatHoursMinutes, formatWaitMinutes, packTimeLines } from '@/utils/format';
import { isActiveStatus, isJobGone } from '@/utils/jobs';
import { looksLikeJson } from '@/utils/progress';
import ArtifactList from './ArtifactList';
import JobError from './JobError';
import MarkdownView from './MarkdownView';
import ProgressBar from './ProgressBar';
import StatusPill from './StatusPill';

interface JobCardProps {
  jobId: string | null;
  onDismiss: () => void;
  job: JobStatusResponse | null;
  /** Ошибка самого запроса статуса (сеть и т.п.) — не ошибка джобы. */
  pollError?: unknown;
  /** Блок с результатом — рендерится при SUCCEEDED (до списка артефактов). */
  successContent?: ReactNode;
}

function pollErrorMessage(pollError: unknown): string {
  if (pollError instanceof Error && pollError.message) {
    return `Не удалось получить статус задачи: ${pollError.message}`;
  }
  return 'Не удалось получить статус задачи';
}

/** Карточка задачи расписания/калибровки/истории: статус, очередь, прогресс, ошибка, итог и файлы. */
export default function JobCard({ jobId, job, pollError, successContent, onDismiss }: JobCardProps) {
  const cancel = useMutation({
    mutationFn: () => {
      if (!jobId) throw new Error('Задача не выбрана');
      return deleteJob(jobId);
    },
    onSuccess: onDismiss,
    onError: (error) => {
      if (error instanceof ApiError && error.status === 404) onDismiss();
      else pushToast(error.message, 'error');
    },
  });
  const cancelButton = (
    <button type="button" className="btn" disabled={cancel.isPending} onClick={() => cancel.mutate()}>
      Отменить задачу
    </button>
  );
  if (isJobGone(pollError)) {
    return (
      <section className="card">
        <div className="error-block">
          Задача больше недоступна: сервер перезапущен, задача удалена или истёк срок хранения.
        </div>
        <button type="button" className="btn" onClick={onDismiss}>
          Закрыть
        </button>
      </section>
    );
  }
  if (pollError && !job) {
    return (
      <section className="card">
        <div className="error-block">{pollErrorMessage(pollError)}</div>
        {jobId && cancelButton}
      </section>
    );
  }

  if (!job) return jobId ? <section className="card">Получаю статус задачи… {cancelButton}</section> : null;

  const active = isActiveStatus(job.status);
  const failed = job.status === 'FAILED';

  return (
    <section className="card job-card">
      <header className="job-card-header">
        <h2 className="job-card-title">
          Задача · <span className="mono">{job.jobId}</span>
        </h2>
        <StatusPill status={job.status} />
      </header>

      {serverMode && job.status === 'QUEUED' && (
        <p>
          Место в очереди: {job.queuePosition ?? '—'}.{' '}
          {job.estimatedWaitFromSeconds != null && job.estimatedWaitToSeconds != null
            ? `Ожидание примерно ${formatWaitMinutes(job.estimatedWaitFromSeconds, job.estimatedWaitToSeconds)} мин. `
            : ''}
          {job.estimateReason ?? 'Время ожидания уточняется.'}
        </p>
      )}
      {serverMode && job.expiresAt && <p className="hint">Файлы доступны до {formatDateTime(job.expiresAt)}</p>}
      {active && job.canManage !== false && cancelButton}
      {active ? (
        <ProgressBar
          percent={job.progressPercent}
          message={job.phaseMessage && !looksLikeJson(job.phaseMessage) ? job.phaseMessage : undefined}
        />
      ) : null}

      {failed ? <JobError error={job.error || 'Задача завершилась с ошибкой'} collapsible /> : null}
      {active && pollError ? <div className="error-block">{pollErrorMessage(pollError)}</div> : null}

      {failed && <ArtifactList artifacts={job.artifacts} />}
      {job.status === 'SUCCEEDED' && (
        <>
          {successContent}
          {job.pack ? <PackSummary pack={job.pack} /> : null}
          {job.calibration ? <CalibrationSummary calibration={job.calibration} /> : null}
          <ArtifactList artifacts={job.artifacts} />
        </>
      )}
    </section>
  );
}

function PackSummary({ pack }: { pack: PackJobInfo }) {
  return (
    <>
      {pack.time ? (
        <div className="stack-sm">
          {packTimeLines(pack.time).map((line) => (
            <p className="time-summary" key={line}>
              {line}
            </p>
          ))}
        </div>
      ) : null}
      {pack.supportsDetected ? (
        <p className="time-summary">обнаружены поддержки: поворот моделей заблокирован</p>
      ) : null}
    </>
  );
}

function CalibrationSummary({ calibration }: { calibration: CalibrationInfo }) {
  return (
    <div className="stack-sm">
      <p className="time-summary">
        Оценка слайсера: A — {formatHoursMinutes(calibration.estimateASeconds)}, B —{' '}
        {formatHoursMinutes(calibration.estimateBSeconds)}; смен в B: {calibration.changesSliced}
      </p>
      <details className="params" open>
        <summary className="params-summary">Инструкция</summary>
        <MarkdownView source={calibration.instructions} />
      </details>
    </div>
  );
}
