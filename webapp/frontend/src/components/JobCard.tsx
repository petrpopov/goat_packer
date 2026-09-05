import type { ReactNode } from 'react';
import type { JobStatusResponse } from '@/api/types';
import ArtifactList from './ArtifactList';
import ProgressBar from './ProgressBar';
import StatusPill from './StatusPill';

interface JobCardProps {
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

export default function JobCard({ job, pollError, successContent }: JobCardProps) {
  if (pollError && !job) {
    return (
      <section className="card">
        <div className="error-block">{pollErrorMessage(pollError)}</div>
      </section>
    );
  }

  if (!job) return null;

  const active = job.status === 'QUEUED' || job.status === 'RUNNING';

  return (
    <section className="card job-card">
      <header className="job-card-header">
        <h2 className="job-card-title">
          Задача · <span className="mono">{job.jobId}</span>
        </h2>
        <StatusPill status={job.status} />
      </header>

      {active && <ProgressBar percent={job.progressPercent} message={job.phaseMessage ?? undefined} />}

      {job.status === 'FAILED' && <div className="error-block">{job.error ?? 'Задача завершилась с ошибкой'}</div>}

      {active && pollError ? <div className="error-block">{pollErrorMessage(pollError)}</div> : null}

      {job.status === 'SUCCEEDED' && (
        <>
          {successContent}
          <ArtifactList artifacts={job.artifacts} />
        </>
      )}
    </section>
  );
}
