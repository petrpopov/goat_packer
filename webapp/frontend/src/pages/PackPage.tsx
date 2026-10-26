import { useEffect, useMemo, useState } from 'react';
import type { FormEvent } from 'react';
import { useMutation, useQuery } from '@tanstack/react-query';
import { ApiError } from '@/api/http';
import { fetchProfiles, startPackJob } from '@/api/services';
import AmsCorrectionFields from '@/components/AmsCorrectionFields';
import FileDrop from '@/components/FileDrop';
import Field from '@/components/Field';
import JobCard from '@/components/JobCard';
import { buildNozzleClean, buildSupportOverlap, buildTimeFields, numberInRange } from '@/utils/forms';
import { useAmsCorrection } from '@/hooks/useAmsCorrection';
import { useProfileDetection } from '@/hooks/useProfileDetection';
import { useJob } from '@/hooks/useJob';
import { pushToast } from '@/store/useToastStore';
import { useUiStore } from '@/store/useUiStore';

type NozzleCleanMode = 'off' | 'global' | 'from-z';

/** Finite deterministic search portfolio; higher effort includes all earlier strategies. */
const EFFORT_LEVELS = [
  { label: 'Быстро', attempts: 500 },
  { label: 'Стандарт', attempts: 2000 },
  { label: 'Сильно', attempts: 4800 },
  { label: 'Максимум', attempts: 12000 },
] as const;

export default function PackPage() {
  const sendToSchedule = useUiStore((state) => state.sendToSchedule);

  const [file, setFile] = useState<File | null>(null);
  const [clearance, setClearance] = useState('0.7');
  const [resolution, setResolution] = useState('0.25');
  const [effort, setEffort] = useState(EFFORT_LEVELS.length - 2);
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
    Boolean(jobId) && !jobGone && (!job || ['QUEUED', 'RUNNING', 'FINALIZING', 'LOST', 'RETRY_WAIT'].includes(job.status));

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
    timeFields = buildTimeFields(
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
    form.append('attempts', String(EFFORT_LEVELS[effort].attempts));
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
          → В расписание
        </button>
      </div>
    ) : null;

  return (
    <div className="page">
      <div>
        <h1 className="page-title">Фаза 1 · Упаковка</h1>
        <p className="page-subtitle">Укладка объектов из .3mf в пределах стола принтера</p>
      </div>

      <form className="stack" onSubmit={handleSubmit}>
        <section className="card">
          <FileDrop file={file} onFileChange={handleFileChange} disabled={jobActive || startMutation.isPending} />
          {detectionError ? <div className="error-block">{detectionError}</div> : null}

          <div className="form-grid">
            <Field
              label="Профиль принтера"
              hint={
                detection === 'detecting'
                  ? 'Определяю принтер по файлу…'
                  : detection === 'ok'
                    ? 'Определён по файлу — выбор заблокирован'
                    : detection === 'unknown'
                      ? 'Профиль не определён'
                      : undefined
              }
            >
              <select className="input" value={profileId} disabled>
                <option value="">{profilesQuery.isPending ? 'Загрузка…' : '— выберите профиль —'}</option>
                {profiles.map((profile) => (
                  <option key={profile.id} value={profile.id}>
                    {profile.name} · {profile.bedWidthMm}×{profile.bedDepthMm} мм
                  </option>
                ))}
              </select>
            </Field>

            <Field
              label="Раскладка"
              hint={
                layout === 'grid' ? 'Ровная сетка, без поиска — быстро' : 'Плотный подбор с поворотами и нависаниями'
              }
            >
              <select
                className="input"
                value={layout}
                onChange={(event) => setLayout(event.target.value as 'search' | 'grid')}
                disabled={jobActive || startMutation.isPending}
              >
                <option value="search">Подбор (плотнее)</option>
                <option value="grid">Сетка (быстро, равномерно)</option>
              </select>
            </Field>

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

            <div className="field">
              <label className="check">
                <input
                  type="checkbox"
                  checked={windowOn}
                  onChange={(event) => setWindowOn(event.target.checked)}
                  disabled={jobActive || startMutation.isPending}
                />
                Уложиться в окно печати
              </label>
              <span className="field-hint">стол урезается до числа объектов, которое печатается за окно</span>
            </div>

            {windowOn && (
              <>
                <Field
                  label="Окно печати, ч"
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
              </>
            )}

            <AmsCorrectionFields
              available={ams.available}
              on={ams.on}
              onToggle={ams.setOn}
              seconds={ams.seconds}
              onSecondsChange={ams.setSeconds}
              invalid={ams.seconds.trim() !== '' && !numberInRange(ams.seconds, 0, 600)}
              disabled={jobActive || startMutation.isPending}
            />

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
          </div>

          <details className="params">
            <summary className="params-summary">Параметры</summary>
            <div className="form-grid">
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
              <Field label="Разрешение растра, мм">
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
                label="Усилие поиска"
                hint={`${EFFORT_LEVELS[effort].attempts} — усилие детерминированного поиска`}
              >
                <div className="effort-slider">
                  <input
                    className="effort-range"
                    type="range"
                    min={0}
                    max={EFFORT_LEVELS.length - 1}
                    step={1}
                    value={effort}
                    onChange={(event) => setEffort(Number(event.target.value))}
                    disabled={jobActive || startMutation.isPending}
                    aria-label="Усилие поиска"
                  />
                  <div className="effort-labels">
                    {EFFORT_LEVELS.map((level, index) => (
                      <span key={level.label} className={index === effort ? 'effort-label active' : 'effort-label'}>
                        {level.label}
                      </span>
                    ))}
                  </div>
                </div>
              </Field>
              <Field
                label="Отступ от края, мм"
                hint={
                  selectedProfile
                    ? `пусто — дефолт профиля (${selectedProfile.edgeMarginMm} мм)`
                    : 'пусто — дефолт профиля'
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
            </div>
          </details>

          <div className="form-actions">
            <button type="submit" className="btn btn-primary" disabled={!canSubmit}>
              {startMutation.isPending ? <span className="spinner" aria-hidden="true" /> : null}
              Упаковать
            </button>
          </div>
        </section>
      </form>

      <JobCard
        jobId={jobId}
        onDismiss={() => setJobId(null)}
        job={job}
        pollError={jobQuery.isError ? jobQuery.error : undefined}
        successContent={successContent}
      />
    </div>
  );
}
