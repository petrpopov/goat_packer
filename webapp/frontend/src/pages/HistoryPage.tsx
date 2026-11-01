import { useEffect, useState } from 'react';
import { useMutation, useQuery } from '@tanstack/react-query';
import { listJobs, listUploads, removeJob, removeUpload } from '@/api/server';
import type { JobKind, JobStatusResponse, UploadedFile } from '@/api/types';
import AmsCalculator from '@/components/AmsCalculator';
import Icon from '@/components/Icon';
import JobCard from '@/components/JobCard';
import Pager from '@/components/Pager';
import StatusPill from '@/components/StatusPill';
import { JOBS_PAGE_SIZE, LIST_POLL_MS, UPLOADS_POLL_MS } from '@/config';
import { useJob } from '@/hooks/useJob';
import { useLinkHandler } from '@/hooks/useLinkHandler';
import { pushToast } from '@/store/useToastStore';
import { useUiStore } from '@/store/useUiStore';
import { calibrationValues } from '@/utils/calibration';
import { formatMebibytes, formatShortDateTime, objectsText } from '@/utils/format';
import { isActiveStatus } from '@/utils/jobs';

const KIND_LABEL: Record<JobKind, string> = { pack: 'Упаковка', schedule: 'Расписание', calibration: 'Калибровка' };
/** Упаковка открывается в своей рабочей области (/pack/<id>), остальное — карточкой здесь. */
const jobHref = (job: JobStatusResponse) => (job.kind === 'pack' ? `/pack/${job.jobId}` : `/jobs/${job.jobId}`);

/** «Мои задачи» (серверный режим): список задач, загруженные исходники и карточка выбранной задачи. */
export default function HistoryPage() {
  // Выбранная задача — это URL (/jobs/:id): ссылку можно скопировать, «назад» закрывает карточку.
  const route = useUiStore((s) => s.route);
  const navigate = useUiStore((s) => s.navigate);
  const selected = route.name === 'job' ? route.id : null;
  const setSelected = (id: string | null) => navigate(id ? `/jobs/${id}` : '/jobs');
  const current = useJob(selected);
  // Старые ссылки /jobs/<id> на упаковку ведут в её рабочую область.
  const currentKind = current.data?.kind;
  useEffect(() => {
    if (selected && currentKind === 'pack') navigate(`/pack/${selected}`, { replace: true });
  }, [selected, currentKind, navigate]);

  return (
    <div className="page">
      <h1 className="page-title">Мои задачи</h1>
      <JobsTable onRemoved={() => setSelected(null)} />
      <UploadsTable />
      <JobCard
        jobId={selected}
        job={current.data ?? null}
        pollError={current.error}
        onDismiss={() => setSelected(null)}
      />
      {current.data?.status === 'SUCCEEDED' && current.data.calibration ? (
        <AmsCalculator
          key={current.data.jobId}
          profileId={current.data.calibration.profileId}
          initial={calibrationValues(current.data.calibration)}
        />
      ) : null}
    </div>
  );
}

function JobsTable({ onRemoved }: { onRemoved: () => void }) {
  const navigate = useUiStore((s) => s.navigate);
  const onLink = useLinkHandler();
  const [page, setPage] = useState(0);
  const list = useQuery({
    queryKey: ['history', page],
    queryFn: () => listJobs(page),
    refetchInterval: LIST_POLL_MS,
  });
  const remove = useMutation({
    mutationFn: removeJob,
    onSuccess: () => {
      onRemoved();
      void list.refetch();
    },
    onError: (e) => pushToast(e.message, 'error'),
  });
  const items = list.data?.items ?? [];

  return (
    <section className="page-section">
      {list.isError && <div className="error-block">{list.error.message}</div>}
      {items.length ? (
        <table className="table table-plain">
          <thead>
            <tr>
              <th>Файл</th>
              <th>Тип</th>
              <th>Статус</th>
              <th>Создана</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {items.map((job) => {
              const href = jobHref(job);
              return (
                <tr
                  key={job.jobId}
                  className="row-link"
                  // Вся строка — ссылка: упаковка открывается в своей рабочей области (/pack/<id>).
                  onClick={(event) => {
                    if (!(event.target as HTMLElement).closest('a, button')) navigate(href);
                  }}
                >
                  <td>
                    <a className="link" href={href} onClick={onLink}>
                      {job.originalName ?? <span className="mono">{job.jobId.slice(0, 8)}</span>}
                    </a>
                    {job.pack ? <div className="row-sub">{objectsText(job.pack.objectCount)} на столе</div> : null}
                  </td>
                  <td>{KIND_LABEL[job.kind] ?? job.kind}</td>
                  <td>
                    <StatusPill status={job.status} />
                  </td>
                  <td className="mono">{formatShortDateTime(job.createdAt)}</td>
                  <td className="row-actions">
                    {!isActiveStatus(job.status) && (
                      <button
                        className="icon-btn"
                        aria-label="Удалить задачу"
                        data-tip="Удалить задачу и её файлы"
                        data-tip-align="end"
                        disabled={remove.isPending}
                        onClick={() => {
                          if (window.confirm('Удалить задачу и ссылки на её результаты?')) remove.mutate(job.jobId);
                        }}
                      >
                        <Icon name="x" />
                      </button>
                    )}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      ) : (
        <p>
          Пока нет задач.{' '}
          <button className="btn" onClick={() => navigate('/pack')}>
            Упаковать файл
          </button>
        </p>
      )}
      <Pager page={page} total={list.data?.total ?? 0} pageSize={JOBS_PAGE_SIZE} onChange={setPage} />
    </section>
  );
}

function UploadsTable() {
  const uploads = useQuery({ queryKey: ['my-uploads'], queryFn: listUploads, refetchInterval: UPLOADS_POLL_MS });
  const remove = useMutation({
    mutationFn: removeUpload,
    onSuccess: () => void uploads.refetch(),
    onError: (e) => pushToast(e.message, 'error'),
  });
  const files: UploadedFile[] = uploads.data ?? [];
  return (
    <section className="page-section">
      <h2>Загруженные исходники</h2>
      <p>Удаление исходника не отменяет уже принятые задачи. Общий blob освободится, когда исчезнут все ссылки.</p>
      {files.length ? (
        <table className="table table-plain">
          <tbody>
            {files.map((file) => (
              <tr key={file.fileId}>
                <td>{file.originalName}</td>
                <td className="mono">{formatMebibytes(file.sizeBytes)}</td>
                <td>{file.state}</td>
                <td className="row-actions">
                  <button
                    className="btn"
                    disabled={remove.isPending || file.state === 'RECEIVING'}
                    onClick={() => {
                      if (window.confirm('Удалить этот исходник из профиля?')) remove.mutate(file.fileId);
                    }}
                  >
                    Удалить
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      ) : null}
    </section>
  );
}
