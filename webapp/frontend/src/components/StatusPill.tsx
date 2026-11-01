import type { JobStatus } from '@/api/types';

const LABELS: Record<JobStatus, string> = {
  QUEUED: 'В очереди',
  RUNNING: 'Выполняется',
  SUCCEEDED: 'Готово',
  FAILED: 'Ошибка',
  FINALIZING: 'Сохранение',
  LOST: 'Нет связи',
  RETRY_WAIT: 'Ожидание повтора',
  CANCELLED: 'Отменено',
  EXPIRED: 'Истекло',
};

const PILL_CLASSES: Record<JobStatus, string> = {
  QUEUED: 'pill pill-muted',
  // Оранжевый — ход машины (DESIGN.md): идущая задача не должна выглядеть как «Готово» (зелёный).
  RUNNING: 'pill pill-run',
  SUCCEEDED: 'pill pill-ok',
  FAILED: 'pill pill-crit',
  FINALIZING: 'pill pill-run',
  LOST: 'pill pill-warn',
  RETRY_WAIT: 'pill pill-warn',
  CANCELLED: 'pill pill-muted',
  EXPIRED: 'pill pill-muted',
};

export default function StatusPill({ status }: { status: JobStatus }) {
  return <span className={PILL_CLASSES[status]}>{LABELS[status]}</span>;
}
