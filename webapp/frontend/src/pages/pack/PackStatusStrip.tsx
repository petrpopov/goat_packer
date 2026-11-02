import { useCallback } from 'react';
import type { ReactNode } from 'react';
import { jobLink } from '@/api/services';
import type { JobStage, JobStatusResponse, PackJobInfo } from '@/api/types';
import Icon from '@/components/Icon';
import StatusStrip, { type StripLine } from '@/components/StatusStrip';
import { pushToast } from '@/store/useToastStore';
import { useUiStore } from '@/store/useUiStore';
import { startDownload } from '@/utils/artifacts';
import { formatBytes, formatHoursMinutes, formatWaitMinutes, objectsText } from '@/utils/format';
import { splitJobError } from '@/utils/jobs';
import { stageClockText, stageElapsed, stripStage } from '@/utils/progress';
import type { PackJob } from './usePackJob';
import type { PrinterSelection } from './usePrinterSelection';

/** id формы слева: «Упаковать» в строке статуса отправляет её через атрибут form. */
export const PACK_FORM_ID = 'pack-form';

const Num = ({ children }: { children: ReactNode }) => <span className="num">{children}</span>;

/** Части второй строки через « · ». */
const joinParts = (parts: ReactNode[]) =>
  parts.map((part, index) => (
    <span key={index}>
      {index ? ' · ' : ''}
      {part}
    </span>
  ));

const lowerFirst = (text: string) => text.charAt(0).toLowerCase() + text.slice(1);

/** Время до конца этапа или сколько он уже длится (если у пакера нет оценки). */
const stageClock = (stage: JobStage, receivedAtMs: number | undefined, now: number) =>
  stageClockText(stageElapsed(stage, receivedAtMs, now), stage.expectedSeconds);

function QueueText({ job, position }: { job: JobStatusResponse; position: number | null | undefined }) {
  if (position == null) return <>ждёт свободного исполнителя</>;
  const wait =
    job.status === 'QUEUED' && job.estimatedWaitFromSeconds != null && job.estimatedWaitToSeconds != null ? (
      <>
        {' '}
        · начнётся через ≈<Num>{formatWaitMinutes(job.estimatedWaitFromSeconds, job.estimatedWaitToSeconds)}</Num> мин
      </>
    ) : null;
  return (
    <>
      место <Num>{position}</Num>
      {wait}
    </>
  );
}

/** Тихая иконка слева от кнопок: ссылкой делятся реже, чем скачивают. */
function CopyLinkButton({ jobId }: { jobId: string }) {
  const copy = useCallback(() => {
    navigator.clipboard
      .writeText(jobLink(jobId))
      .then(() => pushToast('Ссылка скопирована', 'success'))
      .catch(() => pushToast('Не удалось скопировать ссылку', 'error'));
  }, [jobId]);
  return (
    <button
      type="button"
      className="icon-btn copy-quiet"
      aria-label="Скопировать ссылку на задачу"
      data-tip="Скопировать ссылку на задачу"
      data-tip-align="end"
      onClick={copy}
    >
      <Icon name="copy" />
    </button>
  );
}

const Spinner = () => <span className="spinner" aria-hidden="true" />;

// ───── Черновик: файл, его загрузка и определение принтера, готовность ─────

interface DraftStatusProps {
  file: File | null;
  selection: PrinterSelection;
  parsing: boolean;
  canSubmit: boolean;
  submitting: boolean;
}

export function DraftStatusStrip({ file, selection, parsing, canSubmit, submitting }: DraftStatusProps) {
  const { upload, detection, selectedProfile, detectionError } = selection;
  let detail: ReactNode;
  if (!file) detail = 'Загрузите слева проект .3mf или модель .stl';
  else if (upload)
    detail = (
      <>
        Загрузка файла · <Num>{Math.round((upload.sent / Math.max(1, upload.total)) * 100)}</Num> %
      </>
    );
  else if (detection === 'detecting') detail = 'Определяю принтер…';
  else if (parsing) detail = 'Читаю модель…';
  else if (selectedProfile)
    detail = `${selectedProfile.name} · рабочая область ${selectedProfile.bedWidthMm} × ${selectedProfile.bedDepthMm} мм`;
  else detail = detectionError ?? 'Выберите принтер слева';

  const loading = file !== null && (detection === 'detecting' || parsing);
  const actions = loading ? (
    <button type="button" className="btn btn-primary btn-lg btn-main" disabled>
      <Spinner />
      Загрузка…
    </button>
  ) : (
    <button
      type="submit"
      form={PACK_FORM_ID}
      className="btn btn-primary btn-lg btn-main"
      disabled={!canSubmit}
      data-tip={canSubmit ? 'Разложить объекты на столе' : undefined}
      data-tip-align="end"
    >
      {submitting ? <Spinner /> : null}
      Упаковать
    </button>
  );
  return (
    <StatusStrip
      title={<span className="fname">{file ? file.name : 'Модель не выбрана'}</span>}
      detail={detail}
      line={{ kind: 'off' }}
      actions={actions}
      // У черновика «Инфо» есть, но недоступна: этапов и файлов ещё нет, а текст не прыгает при запуске.
      lead={
        <button type="button" className="info-btn" disabled>
          <Icon name="info" />
          Инфо
        </button>
      }
    />
  );
}

// ───── Открытая задача: имя файла и итог (не меняются), что происходит, действия ─────

interface JobStatusProps {
  jobId: string;
  pack: PackJob;
  /** Имя на случай, если статус ещё не пришёл. */
  fallbackName: string;
  now: number;
  infoOpen: boolean;
  onToggleInfo: () => void;
  onOpenInfo: () => void;
  onRepack: () => void;
  repackDisabled: boolean;
}

interface StripContent {
  detail: ReactNode;
  line: StripLine;
  lineLabel?: string;
  actions: ReactNode;
}

export function JobStatusStrip(props: JobStatusProps) {
  const { jobId, pack, fallbackName, infoOpen, onToggleInfo } = props;
  const name = pack.job?.originalName ?? fallbackName;
  // Рядом с именем — сколько объектов получилось (id задачи пользователю ничего не говорит); без итога — ничего.
  const title = (
    <>
      <span className="fname">{name || 'Задача'}</span>
      {pack.resultCount != null ? <span className="job-id">{objectsText(pack.resultCount)}</span> : null}
    </>
  );
  const content = useJobStripContent(props);
  return (
    <StatusStrip
      title={title}
      {...content}
      // «Инфо» слева: у задачи — всегда на месте (и после завершения).
      lead={
        <button
          type="button"
          className="info-btn"
          aria-expanded={infoOpen}
          onClick={onToggleInfo}
          data-tip="Этапы, время и файлы задачи"
        >
          <Icon name="info" />
          Инфо
        </button>
      }
      actions={
        <>
          {pack.gone ? null : <CopyLinkButton jobId={jobId} />}
          {content.actions}
        </>
      }
    />
  );
}

function useJobStripContent({ jobId, pack, now, infoOpen, onOpenInfo, onRepack, repackDisabled }: JobStatusProps) {
  const newDraft = useUiStore((state) => state.newDraft);
  const { job, query } = pack;

  if (pack.gone) {
    return {
      detail: 'Задача недоступна: удалена или истёк срок хранения файлов',
      line: { kind: 'off' },
      actions: (
        <button
          type="button"
          className="btn btn-primary btn-lg btn-main"
          onClick={newDraft}
          data-tip="Начать с чистой формы"
          data-tip-align="end"
        >
          <Icon name="plus" />
          Новая упаковка
        </button>
      ),
    } satisfies StripContent;
  }
  if (!job) {
    return {
      detail: query.isError && query.error instanceof Error ? query.error.message : 'Получаю статус задачи…',
      line: { kind: 'waiting' },
      actions: null,
    } satisfies StripContent;
  }

  const cancel = (
    <button
      type="button"
      className="btn btn-lg btn-main btn-cancel"
      disabled={pack.cancelJob.isPending || job.canManage === false}
      onClick={() => pack.cancelJob.mutate(jobId)}
      data-tip="Остановить и удалить задачу"
      data-tip-align="end"
    >
      <Icon name="stop" />
      Отменить
    </button>
  );
  const status = job.status;

  if (status === 'QUEUED' || status === 'RETRY_WAIT' || status === 'LOST') {
    // Повтор — с причиной от сервера («Сервер перезапускался — задача запущена заново»).
    return {
      detail: joinParts([
        status === 'QUEUED' ? 'В очереди' : (job.phaseMessage ?? 'Задача запущена заново'),
        <QueueText key="queue" job={job} position={job.queuePosition} />,
      ]),
      line: { kind: 'waiting' },
      actions: cancel,
    } satisfies StripContent;
  }

  if (status === 'RUNNING' || status === 'FINALIZING') {
    const stage = job.stage;
    const strip = stage ? stripStage(stage) : null;
    return {
      detail:
        stage && strip
          ? joinParts([
              <>
                Упаковка · этап <Num>{strip.step}</Num> из <Num>{strip.steps}</Num>
              </>,
              lowerFirst(strip.label),
              stageClock(stage, job.receivedAtMs, now),
            ])
          : 'Запуск упаковки…',
      line: { kind: 'progress', percent: job.progressPercent },
      lineLabel: strip ? `${strip.label}, этап ${strip.step} из ${strip.steps}` : undefined,
      actions: cancel,
    } satisfies StripContent;
  }

  if (status === 'SUCCEEDED' && pack.result) {
    return succeededContent(pack, pack.result, now);
  }

  // FAILED / CANCELLED / EXPIRED
  const failed = status === 'FAILED';
  return {
    detail: failed ? (
      <>
        <span className="err">
          Не удалось: {job.error ? splitJobError(job.error).headline : 'задача завершилась с ошибкой'}
        </span>{' '}
        ·{' '}
        <button type="button" className="linkish" aria-expanded={infoOpen} onClick={onOpenInfo}>
          подробности
        </button>
      </>
    ) : status === 'CANCELLED' ? (
      'Задача отменена'
    ) : (
      'Срок хранения истёк'
    ),
    line: { kind: failed ? 'fail' : 'off' },
    actions: (
      <button
        type="button"
        className="btn btn-primary btn-lg btn-main"
        onClick={onRepack}
        disabled={repackDisabled}
        data-tip="Те же файл и параметры — можно поправить перед запуском"
        data-tip-align="end"
      >
        <Icon name="refresh" />
        Упаковать заново
      </button>
    ),
  } satisfies StripContent;
}

/** Текст только дополняется: «Готово: N» → «проверяю нарезкой» → «проверено нарезкой · печать ≈…». */
function succeededContent(pack: PackJob, result: PackJobInfo, now: number): StripContent {
  const job = pack.job!;
  const { check, checkActive, checkRunning, plateFile } = pack;
  const sliced = pack.checkDone || result.finalSliced !== false;
  const parts: ReactNode[] = [
    sliced || checkActive ? (
      <>Готово: {objectsText(result.objectCount)}</>
    ) : (
      <>Готово: {objectsText(result.objectCount)} на столе</>
    ),
  ];
  if (checkActive && check) {
    parts.push(
      checkRunning && check.stage ? (
        <>проверяю нарезкой · {stageClock(check.stage, job.receivedAtMs, now)}</>
      ) : (
        <>
          проверка нарезкой в очереди · <QueueText job={job} position={check.queuePosition} />
        </>
      )
    );
  } else if (sliced) {
    parts.push(<span className="ok">проверено нарезкой</span>);
    if (result.time) {
      parts.push(<>печать ≈{formatHoursMinutes(result.time.correctedSeconds)}</>);
      if (result.time.windowHours != null)
        parts.push(
          <>
            окно <Num>{result.time.windowHours}</Num> ч: <Num>{result.time.chosenCount}</Num> из{' '}
            <Num>{result.time.fullCount}</Num>
          </>
        );
    }
  } else if (check?.status === 'FAILED') {
    parts.push(<span className="err">проверка нарезкой не удалась</span>);
  } else {
    parts.push('нарезкой не проверено');
  }

  const line: StripLine =
    checkRunning && check
      ? { kind: 'progress', percent: check.progressPercent }
      : checkActive
        ? { kind: 'waiting' }
        : { kind: 'ok' };
  const manageable = job.canManage !== false;
  const checkButton = checkActive ? (
    <button
      type="button"
      className="btn btn-lg btn-cancel"
      disabled={pack.cancelCheck.isPending || !manageable}
      onClick={() => pack.cancelCheck.mutate(job.jobId)}
      data-tip="Остановить проверку нарезкой; упакованный стол останется"
      data-tip-align="end"
    >
      <Icon name="stop" />
      Остановить
    </button>
  ) : !sliced ? (
    <button
      type="button"
      className="btn btn-lg"
      disabled={pack.startCheck.isPending || !manageable}
      onClick={() => pack.startCheck.mutate(job.jobId)}
      data-tip="Нарежет весь стол как слайсер: поддержки соседей, края стола, башня, точное время печати. Несколько минут"
      data-tip-align="end"
    >
      {pack.startCheck.isPending ? <Spinner /> : <Icon name="layers" />}
      Проверить нарезкой
    </button>
  ) : null;
  return {
    detail: joinParts(parts),
    line,
    actions: (
      <>
        {checkButton}
        <button
          type="button"
          className="btn btn-primary btn-lg btn-main"
          disabled={!plateFile}
          onClick={() => plateFile && startDownload(plateFile)}
          data-tip={plateFile ? `${plateFile.name} · ${formatBytes(plateFile.sizeBytes)}` : undefined}
          data-tip-align="end"
        >
          <Icon name="download" />
          Скачать 3MF
        </button>
      </>
    ),
  };
}
