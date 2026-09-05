import type { JobStatus } from '@/api/types';

const LABELS: Record<JobStatus, string> = {
  QUEUED: 'В очереди',
  RUNNING: 'Выполняется',
  SUCCEEDED: 'Готово',
  FAILED: 'Ошибка',
};

const PILL_CLASSES: Record<JobStatus, string> = {
  QUEUED: 'pill pill-muted',
  RUNNING: 'pill pill-cyan',
  SUCCEEDED: 'pill pill-ok',
  FAILED: 'pill pill-crit',
};

export default function StatusPill({ status }: { status: JobStatus }) {
  return <span className={PILL_CLASSES[status]}>{LABELS[status]}</span>;
}
