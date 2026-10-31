import { lazy, Suspense, useEffect, useMemo, useState } from 'react';
import type { FormEvent } from 'react';
import { useMutation, useQuery } from '@tanstack/react-query';
import { ApiError } from '@/api/http';
import { fetchProfiles, startPackJob } from '@/api/services';
import AmsCorrectionFields from '@/components/AmsCorrectionFields';
import FileDrop from '@/components/FileDrop';
import Field from '@/components/Field';
import Icon from '@/components/Icon';
import JobCard from '@/components/JobCard';
import CalibrationPage from '@/pages/CalibrationPage';
import { sceneLayout } from '@/scene/layout';
import { parseModel, type ModelMesh } from '@/scene/model3d';
import { buildNozzleClean, buildSupportOverlap, numberInRange, packTimeFields } from '@/utils/forms';
import { objectsText, sceneClip } from '@/utils/progress';
import { useAmsCorrection } from '@/hooks/useAmsCorrection';
import { useProfileDetection } from '@/hooks/useProfileDetection';
import { useJob } from '@/hooks/useJob';
import { pushToast } from '@/store/useToastStore';
import { useUiStore } from '@/store/useUiStore';

type NozzleCleanMode = 'off' | 'global' | 'from-z';

// three.js тяжёлый: сцена грузится отдельным чанком, только когда известен профиль.
const BedScene = lazy(() => import('@/scene/BedScene'));

export default function PackPage() {
  const sendToSchedule = useUiStore((state) => state.sendToSchedule);
  const calibOpen = useUiStore((state) => state.calibOpen);
  const setCalibOpen = useUiStore((state) => state.setCalibOpen);

  const [file, setFile] = useState<File | null>(null);
  const [clearance, setClearance] = useState('0.7');
  const [resolution, setResolution] = useState('0.25');
  const [cleanMode, setCleanMode] = useState<NozzleCleanMode>('off');
  const [layout, setLayout] = useState<'search' | 'grid'>('search');
  const [edgeMargin, setEdgeMargin] = useState('');
  const [globalEvery, setGlobalEvery] = useState('10');
  const [fromZ, setFromZ] = useState('');
  const [fromZEvery, setFromZEvery] = useState('1');
  const [windowOn, setWindowOn] = useState(false);
  const [windowHours, setWindowHours] = useState('12');
  const [marginPct, setMarginPct] = useState('10');
  const [supportOverlap, setSupportOverlap] = useState('');
  const [jobId, setJobId] = useState<string | null>(null);
  const [model, setModel] = useState<ModelMesh | null>(null);

  // Предпросмотр модели в сцене разбирается в браузере из того же файла, что уйдёт на сервер.
  useEffect(() => {
    let cancelled = false;
    setModel(null);
    if (!file) return;
    file
      .arrayBuffer()
      .then((buffer) => {
        if (!cancelled) setModel(parseModel(file.name, buffer));
      })
      .catch((error: unknown) => {
        if (!cancelled) pushToast(error instanceof Error ? error.message : 'Не удалось прочитать модель', 'error');
      });
    return () => {
      cancelled = true;
    };
  }, [file]);

  // Калибровка нужна только для коррекции внутри окна: без окна панель закрывается.
  useEffect(() => {
    if (!windowOn) setCalibOpen(false);
  }, [windowOn, setCalibOpen]);

  const profilesQuery = useQuery({ queryKey: ['profiles'], queryFn: fetchProfiles });
  const profiles = useMemo(() => profilesQuery.data ?? [], [profilesQuery.data]);

  useEffect(() => {
    if (profilesQuery.isError) {
      pushToast(
        profilesQuery.error instanceof ApiError ? profilesQuery.error.message : 'Не удалось загрузить профили',
        'error'
      );
    }
  }, [profilesQuery.isError, profilesQuery.error]);

  const detectMutation = useProfileDetection(file);
  const profileId = detectMutation.data?.profileId ?? '';
  const handleFileChange = (next: File | null) => setFile(next);

  const jobQuery = useJob(jobId);
  const job = jobQuery.data ?? null;
  const jobGone = jobQuery.error instanceof ApiError && jobQuery.error.status === 404;
  const jobActive =
    Boolean(jobId) &&
    !jobGone &&
    (!job || ['QUEUED', 'RUNNING', 'FINALIZING', 'LOST', 'RETRY_WAIT'].includes(job.status));

  const selectedProfile = profiles.find((profile) => profile.id === profileId) ?? null;
  const nozzleCleanAvailable = selectedProfile === null || selectedProfile.supportsNozzleClean;
  const ams = useAmsCorrection(selectedProfile);

  // Состояние детекта профиля по загруженному файлу: idle / detecting / ok / unknown.
  const detection =
    file === null
      ? 'idle'
      : detectMutation.isPending
        ? 'detecting'
        : detectMutation.data?.profileId != null
          ? 'ok'
          : 'unknown';
  const detectionError = detectMutation.isError
    ? detectMutation.error instanceof Error
      ? detectMutation.error.message
      : 'Не удалось проверить файл'
    : detection === 'unknown'
      ? detectMutation.data?.printerModel
        ? `Для принтера «${detectMutation.data.printerModel}» не найден подходящий профиль.`
        : 'В проекте не указан printer_model. Сохраните проект с выбранным принтером в слайсере.'
      : null;

  const startMutation = useMutation({
    mutationFn: startPackJob,
    onSuccess: (response) => {
      setJobId(response.jobId);
      pushToast('Задача упаковки запущена', 'success');
    },
    onError: (error) => {
      pushToast(error instanceof ApiError ? error.message : 'Не удалось выполнить запрос', 'error');
    },
  });

  const effectiveCleanMode = nozzleCleanAvailable ? cleanMode : 'off';
  let nozzleSpec = '';
  let nozzleCleanInvalid = false;
  try {
    nozzleSpec = buildNozzleClean(effectiveCleanMode, fromZ, globalEvery, fromZEvery);
  } catch {
    nozzleCleanInvalid = true;
  }
  let timeFields: [string, string][] = [];
  let timeInvalid = false;
  try {
    // Коррекция смены прутка влияет только на окно печати: без окна она не шлётся.
    timeFields = packTimeFields(
      { windowOn, windowHours, marginPct, amsOn: ams.on, changeSeconds: ams.seconds },
      ams.available
    );
  } catch {
    timeInvalid = true;
  }
  let supportFields: [string, string][] = [];
  let supportInvalid = false;
  try {
    supportFields = buildSupportOverlap(supportOverlap);
  } catch {
    supportInvalid = true;
  }
  const numbersValid =
    numberInRange(clearance, 0.05, 10) &&
    numberInRange(resolution, 0.05, 1) &&
    (edgeMargin.trim() === '' || numberInRange(edgeMargin, 0, 20));
  const canSubmit =
    Boolean(file) &&
    selectedProfile !== null &&
    numbersValid &&
    detection === 'ok' &&
    !jobActive &&
    !startMutation.isPending &&
    !nozzleCleanInvalid &&
    !timeInvalid &&
    !supportInvalid;

  const handleSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!canSubmit || !file) return;

    const form = new FormData();
    form.append('file', file);
    form.append('profileId', profileId);
    form.append('clearance', clearance);
    form.append('resolution', resolution);
    // attempts не шлём: действует дефолт бэкенда (= CLI). Ползунок усилия убран — с поддержками
    // поиск всегда перебирает фиксированные 64 варианта, число попыток ни на что не влияло.
    form.append('nozzleClean', nozzleSpec);
    form.append('layout', layout);
    // Пустой отступ = дефолт профиля (у Snapmaker U1 — 4 мм против качания головки).
    if (edgeMargin.trim() !== '') form.append('edgeMargin', edgeMargin.trim());
    // Окно и коррекция — только если заданы: иначе действуют дефолты CLI.
    for (const [name, value] of timeFields) form.append(name, value);
    // Пустое перекрытие = без ограничений: поле не шлётся, флаг CLI не передаётся.
    for (const [name, value] of supportFields) form.append(name, value);

    startMutation.mutate(form);
  };

  const successContent =
    job?.status === 'SUCCEEDED' && job.pack ? (
      <div className="form-actions">
        <p className="result-title">Упаковано объектов: {job.pack.objectCount}</p>
        <button
          type="button"
          className="btn btn-primary"
          onClick={() => sendToSchedule({ packJobId: job.jobId, profileId: job.pack?.profileId ?? profileId })}
        >
          В расписание <Icon name="arrow-right" />
        </button>
      </div>
    ) : null;

  const busy = jobActive || startMutation.isPending;
  const running = job?.status === 'RUNNING' || job?.status === 'FINALIZING';
  // Ссылка на layout стабильна между опросами (structural sharing react-query) — сцена не перестраивается зря.
  const jobLayout = job?.layout;
  const placed = useMemo(() => sceneLayout(jobLayout), [jobLayout]);
  const sceneMode = running ? 'running' : job?.status === 'SUCCEEDED' ? 'done' : 'idle';

  const nozzleCleanFields = (
    <>
      <Field
        label="Чистка сопла"
        hint={nozzleCleanAvailable ? undefined : 'Принтер без кода очистки — чистка недоступна'}
      >
        <select
          className="input"
          value={cleanMode}
          onChange={(event) => setCleanMode(event.target.value as NozzleCleanMode)}
          disabled={!nozzleCleanAvailable || jobActive}
        >
          <option value="off">Выкл</option>
          <option value="global">Всегда (каждый N-й слой)</option>
          <option value="from-z">С высоты H мм</option>
        </select>
      </Field>

      {cleanMode === 'global' && (
        <Field label="Каждый N слой">
          <input
            className="input"
            type="number"
            min={1}
            step={1}
            value={globalEvery}
            onChange={(event) => setGlobalEvery(event.target.value)}
            disabled={jobActive || startMutation.isPending}
          />
        </Field>
      )}

      {cleanMode === 'from-z' && (
        <>
          <Field
            label="Высота H, мм"
            error={nozzleCleanInvalid}
            hint={nozzleCleanInvalid ? 'Укажите высоту' : undefined}
          >
            <input
              className="input"
              type="number"
              min={0}
              step="any"
              value={fromZ}
              onChange={(event) => setFromZ(event.target.value)}
              disabled={jobActive || startMutation.isPending}
            />
          </Field>
          <Field label="Каждый N слой">
            <input
              className="input"
              type="number"
              min={1}
              step={1}
              value={fromZEvery}
              onChange={(event) => setFromZEvery(event.target.value)}
              disabled={jobActive || startMutation.isPending}
            />
          </Field>
        </>
      )}
    </>
  );
  const windowFields = (
    <div className="row2">
      <Field
        label="Окно, ч"
        error={!numberInRange(windowHours, 0.5, 240)}
        hint={numberInRange(windowHours, 0.5, 240) ? undefined : '0.5–240 ч'}
      >
        <input
          className="input"
          type="number"
          min={0.5}
          max={240}
          step="any"
          value={windowHours}
          onChange={(event) => setWindowHours(event.target.value)}
          disabled={jobActive || startMutation.isPending}
        />
      </Field>
      <Field
        label="Запас, %"
        error={!numberInRange(marginPct, 0, 50)}
        hint={numberInRange(marginPct, 0, 50) ? 'добавляется к оценке слайсера' : '0–50 %'}
      >
        <input
          className="input"
          type="number"
          min={0}
          max={50}
          step="any"
          value={marginPct}
          onChange={(event) => setMarginPct(event.target.value)}
          disabled={jobActive || startMutation.isPending}
        />
      </Field>
    </div>
  );
  const supportOverlapField = (
    <Field
      label="Перекрытие поддержек, мм"
      error={supportInvalid}
      hint={
        supportInvalid
          ? '0–20 мм или пусто'
          : 'пусто — без ограничений, 0 — поддержки соседей не пересекаются; тело модели никогда не перекрывается'
      }
    >
      <input
        className="input"
        type="number"
        min={0}
        max={20}
        step="any"
        placeholder="без ограничений"
        value={supportOverlap}
        onChange={(event) => setSupportOverlap(event.target.value)}
        disabled={jobActive || startMutation.isPending}
      />
    </Field>
  );
  const paramsFields = (
    <>
      <Field label="Зазор, мм">
        <input
          className="input"
          type="number"
          min={0.05}
          step="any"
          value={clearance}
          onChange={(event) => setClearance(event.target.value)}
          disabled={jobActive || startMutation.isPending}
        />
      </Field>
      <Field label="Растр, мм">
        <input
          className="input"
          type="number"
          min={0.05}
          step="any"
          value={resolution}
          onChange={(event) => setResolution(event.target.value)}
          disabled={jobActive || startMutation.isPending}
        />
      </Field>
      <Field
        label="Отступ от края, мм"
        hint={
          selectedProfile ? `пусто — дефолт профиля (${selectedProfile.edgeMarginMm} мм)` : 'пусто — дефолт профиля'
        }
      >
        <input
          className="input"
          type="number"
          min={0}
          step="any"
          placeholder={selectedProfile ? String(selectedProfile.edgeMarginMm) : ''}
          value={edgeMargin}
          onChange={(event) => setEdgeMargin(event.target.value)}
          disabled={jobActive || startMutation.isPending}
        />
      </Field>
    </>
  );

  return (
    <div className={`work${calibOpen ? ' with-side' : ''}`}>
      <form id="pack-form" className="form-col" onSubmit={handleSubmit}>
        <FileDrop file={file} onFileChange={handleFileChange} disabled={busy} />
        {detectionError ? <div className="error-block">{detectionError}</div> : null}
        {/* Профиль не выбирается: его определяет printer_model в файле, сервер примет только его. */}
        <Field
          label="Профиль принтера"
          hint={
            detection === 'detecting'
              ? 'Определяю принтер по файлу…'
              : detection === 'ok'
                ? 'Определён по файлу'
                : detection === 'unknown'
                  ? 'Профиль не определён'
                  : undefined
          }
        >
          <div className="input" aria-readonly="true">
            {selectedProfile ? selectedProfile.name : profilesQuery.isPending ? 'Загрузка…' : '—'}
          </div>
        </Field>
        <Field
          label="Раскладка"
          hint={layout === 'grid' ? 'быстро и равномерно, без поворотов' : 'подбор с поворотами и нависаниями'}
        >
          <div className="seg" role="group" aria-label="Раскладка">
            <button
              type="button"
              aria-pressed={layout === 'search'}
              onClick={() => setLayout('search')}
              disabled={busy}
            >
              Плотная
            </button>
            <button type="button" aria-pressed={layout === 'grid'} onClick={() => setLayout('grid')} disabled={busy}>
              Сетка
            </button>
          </div>
        </Field>
        {nozzleCleanFields}
        <div className="form-group">
          <label className="check">
            <input
              type="checkbox"
              checked={windowOn}
              onChange={(event) => setWindowOn(event.target.checked)}
              disabled={busy}
            />
            <span className="check-text">
              Уложиться в окно печати
              <small className="field-hint">без окна стол заполняется максимально</small>
            </span>
          </label>
          {windowOn && (
            <div className="nested">
              {windowFields}
              <AmsCorrectionFields
                available={ams.available}
                on={ams.on}
                onToggle={ams.setOn}
                seconds={ams.seconds}
                onSecondsChange={ams.setSeconds}
                invalid={ams.seconds.trim() !== '' && !numberInRange(ams.seconds, 0, 600)}
                disabled={busy}
                onCalibrate={() => setCalibOpen(!calibOpen)}
              />
            </div>
          )}
        </div>
        <details className="params form-group">
          <summary className="params-summary">Дополнительно</summary>
          {supportOverlapField}
          {paramsFields}
        </details>
      </form>

      <section className="main-col">
        <div className="action-bar">
          <div className="title">
            <h1>{file ? file.name : 'Модель не выбрана'}</h1>
            <p>
              {selectedProfile
                ? `${selectedProfile.name} · рабочая область ${selectedProfile.bedWidthMm} × ${selectedProfile.bedDepthMm} мм`
                : 'Загрузите слева проект .3mf с одним объектом'}
            </p>
          </div>
          {job?.status === 'SUCCEEDED' ? (
            <button type="submit" form="pack-form" className="btn" disabled={!canSubmit}>
              Упаковать заново
            </button>
          ) : (
            <button type="submit" form="pack-form" className="btn btn-primary btn-lg" disabled={!canSubmit}>
              {startMutation.isPending ? <span className="spinner" aria-hidden="true" /> : null}
              Упаковать
            </button>
          )}
        </div>
        <JobCard
          jobId={jobId}
          onDismiss={() => setJobId(null)}
          job={job}
          pollError={jobQuery.isError ? jobQuery.error : undefined}
          successContent={successContent}
        />
        {selectedProfile && (
          <Suspense fallback={<div className="scene" />}>
            <BedScene
              profile={selectedProfile}
              model={model}
              mode={sceneMode}
              clip={running && job?.stage ? sceneClip(job.stage.id, job.progressPercent) : 1}
              caption={
                sceneMode === 'done' ? 'перетащите, чтобы повернуть' : model ? 'модель и её проекция на стол' : ''
              }
              placements={placed.placements}
              footprintLoops={placed.footprintLoops}
              tower={placed.tower}
              readout={
                placed.placements.length
                  ? `${objectsText(placed.placements.length)} на столе`
                  : model
                    ? `${model.size.map((v) => Math.round(v)).join(' × ')} мм`
                    : ''
              }
            />
          </Suspense>
        )}
      </section>

      {calibOpen && (
        <aside className="side-col">
          <CalibrationPage embedded onClose={() => setCalibOpen(false)} />
        </aside>
      )}
    </div>
  );
}
