import { useMutation } from '@tanstack/react-query';
import { ApiError, serverMode } from '@/api/http';
import { deleteJob } from '@/api/services';
import { pushToast } from '@/store/useToastStore';
import type { ReactNode } from 'react';
import type { JobStatusResponse } from '@/api/types';
import { formatHoursMinutes, packTimeLines } from '@/utils/format';
import { looksLikeJson } from '@/utils/progress';
import ArtifactList from './ArtifactList';
import MarkdownView from './MarkdownView';
import PackProgress from './PackProgress';
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

/** Разделитель «заголовок / сырые детали» из CliErrorTranslator бэкенда. */
const DETAILS_SEPARATOR = '\n— детали —\n';

/**
 * Ошибка джобы: человекопонятный заголовок крупно, сырой хвост журнала — свёрнутым блоком
 * (бэкенд склеивает их через DETAILS_SEPARATOR; без разделителя показываем как раньше).
 */
function JobError({ error }: { error: string }) {
  const splitAt = error.indexOf(DETAILS_SEPARATOR);
  const headline = splitAt >= 0 ? error.slice(0, splitAt) : error;
  const details = splitAt >= 0 ? error.slice(splitAt + DETAILS_SEPARATOR.length) : null;
  return (
    <>
      <div className="error-block">{headline}</div>
      {details ? (
        <details className="params">
          <summary className="params-summary">Подробности журнала</summary>
          <pre className="error-details">{details}</pre>
        </details>
      ) : null}
    </>
  );
}

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
  if (pollError instanceof ApiError && [404, 410].includes(pollError.status)) {
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

  const active = ['QUEUED', 'RUNNING', 'FINALIZING', 'LOST', 'RETRY_WAIT'].includes(job.status);

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
            ? `Ожидание примерно ${Math.floor(job.estimatedWaitFromSeconds / 60)}–${Math.ceil(job.estimatedWaitToSeconds / 60)} мин. `
            : ''}
          {job.estimateReason ?? 'Время ожидания уточняется.'}
        </p>
      )}
      {serverMode && job.expiresAt && (
        <p className="hint">Файлы доступны до {new Date(job.expiresAt).toLocaleString()}</p>
      )}
      {serverMode && sessionStorage.getItem(`return:${job.jobId}`) && (
        <div className="stack-sm">
          <span>Сохраните ссылку для возврата (даёт доступ к просмотру):</span>
          <input className="input mono" readOnly value={sessionStorage.getItem(`return:${job.jobId}`) ?? ''} />
        </div>
      )}
      {active && job.canManage !== false && cancelButton}
      {active && job.kind === 'pack' && job.stage ? (
        <PackProgress
          percent={job.progressPercent}
          stage={job.stage}
          message={job.phaseMessage}
          receivedAtMs={job.receivedAtMs}
        />
      ) : active ? (
        <ProgressBar
          percent={job.progressPercent}
          message={job.phaseMessage && !looksLikeJson(job.phaseMessage) ? job.phaseMessage : undefined}
        />
      ) : null}

      {job.status === 'FAILED' && job.error ? <JobError error={job.error} /> : null}
      {job.status === 'FAILED' && !job.error ? <div className="error-block">Задача завершилась с ошибкой</div> : null}

      {active && pollError ? <div className="error-block">{pollErrorMessage(pollError)}</div> : null}

      {job.status === 'FAILED' && <ArtifactList artifacts={job.artifacts} />}
      {job.status === 'SUCCEEDED' && (
        <>
          {successContent}
          {job.pack?.time ? (
            <div className="stack-sm">
              {packTimeLines(job.pack.time).map((line) => (
                <p className="time-summary" key={line}>
                  {line}
                </p>
              ))}
            </div>
          ) : null}
          {job.pack?.supportsDetected ? (
            <p className="time-summary">обнаружены поддержки: поворот моделей заблокирован</p>
          ) : null}
          {job.calibration ? (
            <div className="stack-sm">
              <p className="time-summary">
                Оценка слайсера: A — {formatHoursMinutes(job.calibration.estimateASeconds)}, B —{' '}
                {formatHoursMinutes(job.calibration.estimateBSeconds)}; смен в B: {job.calibration.changesSliced}
              </p>
              <details className="params" open>
                <summary className="params-summary">Инструкция</summary>
                <MarkdownView source={job.calibration.instructions} />
              </details>
            </div>
          ) : null}
          <ArtifactList artifacts={job.artifacts} />
        </>
      )}
    </section>
  );
}
