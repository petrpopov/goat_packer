import type { JobArtifact, JobStage, JobStatusResponse } from '@/api/types';
import JobError from '@/components/JobError';
import JobFiles from '@/components/JobFiles';
import ScenePopover from '@/components/ScenePopover';
import { PACK_LOG } from '@/utils/artifacts';
import { formatDuration, objectsText } from '@/utils/format';
import { PRELIMINARY_HINT, isActiveStatus } from '@/utils/jobs';
import { stageElapsed, stageRows } from '@/utils/progress';
import type { PackJob } from './usePackJob';

/** Панель «Инфо» поверх сцены: этапы задачи и проверки нарезкой с длительностями, ошибка, файлы. */
export default function PackInfoPanel({
  jobId,
  job,
  pack,
  now,
  onClose,
}: {
  jobId: string;
  job: JobStatusResponse;
  pack: PackJob;
  now: number;
  onClose: () => void;
}) {
  const { check, checkActive, checkRunning, checkDone } = pack;
  const active = isActiveStatus(job.status);
  return (
    <ScenePopover title={`Задача #${jobId.slice(0, 8)}`} onClose={onClose}>
      {job.status === 'RETRY_WAIT' || job.status === 'LOST' ? (
        <p className="hint info-note">{job.phaseMessage ?? 'Задача запущена заново'}: этапы начнутся сначала.</p>
      ) : null}
      {job.stage ? (
        <StageList
          stage={job.stage}
          running={pack.running}
          failed={job.status === 'FAILED'}
          elapsedNow={stageElapsed(job.stage, job.receivedAtMs, now)}
          finishedAt={job.finishedAt}
          layoutObjects={pack.objectCount}
          layoutPreliminary={pack.countPreliminary}
        />
      ) : (
        <p className="hint">{active ? 'Этапы появятся, когда задача начнётся.' : 'Этапов нет.'}</p>
      )}
      {job.status === 'FAILED' && job.error ? <JobError error={job.error} /> : null}
      {check ? (
        <>
          <div className="group-title">Проверка нарезкой</div>
          {check.stage ? (
            <StageList
              stage={check.stage}
              running={checkRunning}
              failed={check.status === 'FAILED'}
              elapsedNow={stageElapsed(check.stage, job.receivedAtMs, now)}
              finishedAt={check.finishedAt}
            />
          ) : (
            <p className="hint">
              {checkActive ? 'В очереди.' : check.status === 'CANCELLED' ? 'Остановлена.' : 'Этапов нет.'}
            </p>
          )}
          {check.status === 'FAILED' && check.error ? <JobError error={check.error} /> : null}
        </>
      ) : null}
      {!active ? (
        <>
          <div className="group-title">Файлы</div>
          <JobFiles
            artifacts={
              checkDone && check
                ? [...job.artifacts.filter((a) => a.name === PACK_LOG), ...check.artifacts]
                : job.artifacts
            }
            labelOf={(artifact: JobArtifact) =>
              checkDone && artifact.name === PACK_LOG
                ? job.artifacts.includes(artifact)
                  ? 'Журнал упаковки'
                  : 'Журнал проверки нарезкой'
                : undefined
            }
            expiresAt={job.expiresAt}
          />
        </>
      ) : null}
    </ScenePopover>
  );
}

/** Этапы с длительностями: пройденные, текущий (со спиннером и тикающим временем) и ещё предстоящие. */
function StageList({
  stage,
  running,
  failed,
  elapsedNow,
  finishedAt,
  layoutObjects = null,
  layoutPreliminary = false,
}: {
  stage: JobStage;
  running: boolean;
  failed: boolean;
  elapsedNow: number;
  finishedAt: string | null;
  /** Объектов в раскладке — у последнего этапа раскладки, как только он начался. */
  layoutObjects?: number | null;
  layoutPreliminary?: boolean;
}) {
  const rows = stageRows(stage, {
    running,
    failed,
    elapsedNow,
    finishedAtMs: finishedAt ? Date.parse(finishedAt) : null,
  });
  const lastLayoutRow = rows.reduce(
    (last, row, index) => (row.state !== 'pending' && isLayoutStage(row.key) ? index : last),
    -1
  );
  return (
    <ol className="stage-list">
      {rows.map((row, index) => (
        <li key={row.key} className={row.state}>
          {row.state === 'current' ? <span className="spin" aria-hidden="true" /> : <i aria-hidden="true" />}
          {row.label}
          {index === lastLayoutRow && layoutObjects != null ? (
            <span className="stage-note" title={layoutPreliminary ? PRELIMINARY_HINT : undefined}>
              {layoutPreliminary ? '≈ ' : ''}
              {objectsText(layoutObjects)}
            </span>
          ) : null}
          <span className="t">{row.seconds == null ? '' : formatDuration(row.seconds)}</span>
        </li>
      ))}
    </ol>
  );
}

/** Этапы, после которых у пакера есть раскладка: строка «Раскладка»/«Перепаковка» (ключ — id этапа). */
function isLayoutStage(rowKey: string): boolean {
  const id = rowKey.split(':')[0];
  return id === 'packing' || id === 'repacking';
}
