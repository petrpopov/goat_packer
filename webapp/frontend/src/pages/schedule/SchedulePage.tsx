import { useEffect, useState } from 'react';
import type { FormEvent } from 'react';
import { useMutation } from '@tanstack/react-query';
import { startScheduleJob } from '@/api/services';
import AmsCorrectionFields from '@/components/AmsCorrectionFields';
import Field from '@/components/Field';
import FileDrop from '@/components/FileDrop';
import JobCard from '@/components/JobCard';
import NumberInput from '@/components/NumberInput';
import { useAmsCorrection } from '@/hooks/useAmsCorrection';
import { useJobView } from '@/hooks/useJob';
import { useProfileDetection } from '@/hooks/useProfileDetection';
import { useProfiles } from '@/hooks/useProfiles';
import { pushToast } from '@/store/useToastStore';
import { useUiStore } from '@/store/useUiStore';
import { buildTimeFields, clockSeconds, inRange, LIMITS, validTimeWindow } from '@/utils/forms';
import { apiErrorMessage } from '@/utils/jobs';
import ScheduleSummary from './ScheduleSummary';

type SourceKind = 'pack' | 'file';
type BenchMode = 'adaptive' | 'full';

const FORM_ID = 'schedule-form';
/** Время дня, ЧЧ:ММ; часы до 47 — горизонт может уходить в следующие сутки. */
const DEFAULT_CLOCKS = { dayStart: '10:00', nightStart: '23:00', horizonEnd: '34:00' };
const DEFAULT_MINUTES = '10';

/** Расписание на сутки: упакованный стол (из упаковки или файлом) → партии по времени и мультипластинный 3MF. */
export default function SchedulePage() {
  const packHandoff = useUiStore((state) => state.packHandoff);

  const [source, setSource] = useState<SourceKind>(packHandoff ? 'pack' : 'file');
  const [file, setFile] = useState<File | null>(null);
  const detection = useProfileDetection(source === 'file' ? file : null);
  const profileId = source === 'pack' ? (packHandoff?.profileId ?? '') : (detection.data?.profileId ?? '');
  useEffect(() => {
    if (packHandoff) setSource('pack');
  }, [packHandoff]);
  const [clocks, setClocks] = useState(DEFAULT_CLOCKS);
  const [turnaroundMin, setTurnaroundMin] = useState(DEFAULT_MINUTES);
  const [prepMin, setPrepMin] = useState(DEFAULT_MINUTES);
  const [benchMode, setBenchMode] = useState<BenchMode>('adaptive');
  const [jobId, setJobId] = useState<string | null>(null);

  const { profiles, loading: profilesLoading } = useProfiles(true);
  const selectedProfile = profiles.find((p) => p.id === profileId) ?? null;
  const ams = useAmsCorrection(selectedProfile);
  let timeFields: [string, string][] = [];
  let timeInvalid = false;
  try {
    // В расписании окна нет — только коррекция смены прутка.
    timeFields = buildTimeFields(
      { windowOn: false, windowHours: '', marginPct: '', amsOn: ams.on, changeSeconds: ams.seconds },
      ams.available
    );
  } catch {
    timeInvalid = true;
  }

  const { query: jobQuery, job, active: jobActive } = useJobView(jobId);
  const timesValid = validTimeWindow(clocks.dayStart, clocks.nightStart, clocks.horizonEnd);

  const startMutation = useMutation({
    mutationFn: startScheduleJob,
    onSuccess: (response) => {
      setJobId(response.jobId);
      pushToast('Задача расписания запущена', 'success');
    },
    onError: (error) => pushToast(apiErrorMessage(error, 'Не удалось выполнить запрос'), 'error'),
  });
  const busy = jobActive || startMutation.isPending;

  const sourceReady = source === 'pack' ? packHandoff !== null : file !== null;
  const canSubmit =
    sourceReady &&
    selectedProfile !== null &&
    timesValid &&
    inRange(turnaroundMin, LIMITS.scheduleMinutes, true) &&
    inRange(prepMin, LIMITS.scheduleMinutes, true) &&
    !timeInvalid &&
    !busy;

  const handleSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!canSubmit) return;
    if (source === 'pack' ? !packHandoff : !file) return;

    const form = new FormData();
    if (source === 'pack' && packHandoff) form.append('packJobId', packHandoff.packJobId);
    else if (file) form.append('file', file);
    form.append('profileId', profileId);
    form.append('dayStart', clocks.dayStart.trim());
    form.append('nightStart', clocks.nightStart.trim());
    form.append('horizonEnd', clocks.horizonEnd.trim());
    form.append('turnaroundMin', turnaroundMin);
    form.append('prepMin', prepMin);
    form.append('benchMode', benchMode);
    for (const [name, value] of timeFields) form.append(name, value);
    startMutation.mutate(form);
  };

  const clockField = (key: keyof typeof DEFAULT_CLOCKS, label: string, hint?: string) => {
    const valid = clockSeconds(clocks[key]) !== null;
    return (
      <Field label={label} error={!valid} hint={valid ? hint : 'Формат ЧЧ:ММ'}>
        <input
          className="input mono"
          type="text"
          placeholder={DEFAULT_CLOCKS[key]}
          value={clocks[key]}
          onChange={(event) => setClocks((current) => ({ ...current, [key]: event.target.value }))}
          disabled={busy}
        />
      </Field>
    );
  };
  const minutesField = (label: string, value: string, onChange: (value: string) => void) => (
    <Field label={label}>
      <NumberInput
        min={LIMITS.scheduleMinutes.min}
        max={LIMITS.scheduleMinutes.max}
        step={1}
        value={value}
        onChange={onChange}
        disabled={busy}
      />
    </Field>
  );

  return (
    <div className="work">
      <form id={FORM_ID} className="form-col" onSubmit={handleSubmit}>
        <section className="stack">
          <div className="source-switch">
            {packHandoff && (
              <button
                type="button"
                className={`btn${source === 'pack' ? ' btn-primary' : ''}`}
                disabled={busy}
                onClick={() => setSource('pack')}
              >
                Из упаковки
              </button>
            )}
            <button
              type="button"
              className={`btn${source === 'file' ? ' btn-primary' : ''}`}
              disabled={busy}
              onClick={() => setSource('file')}
            >
              Файл .3mf
            </button>
          </div>

          {source === 'pack' && packHandoff ? (
            <p className="source-note">
              Результат упаковки: job <span className="mono">{packHandoff.packJobId}</span>
              <button type="button" className="btn-ghost" disabled={busy} onClick={() => setSource('file')}>
                Заменить файлом
              </button>
            </p>
          ) : (
            <FileDrop file={file} onFileChange={setFile} disabled={busy} />
          )}

          {source === 'file' && detection.isError && <div className="error-block">{detection.error.message}</div>}
          {source === 'file' && file && detection.isSuccess && !profileId && (
            <div className="error-block">Не найден профиль принтера в файле</div>
          )}
          {!timesValid && (
            <div className="error-block">Часы 0–47; начало дня должно быть раньше ночи, а ночь — раньше горизонта.</div>
          )}
          {/* Колонка формы узкая: поля идут одной колонкой. */}
          <div className="stack">
            <Field label="Профиль принтера">
              <select className="input" value={profileId} disabled>
                <option value="">{profilesLoading ? 'Загрузка…' : '— выберите профиль —'}</option>
                {profiles.map((profile) => (
                  <option key={profile.id} value={profile.id}>
                    {profile.name} · {profile.bedWidthMm}×{profile.bedDepthMm} мм
                  </option>
                ))}
              </select>
            </Field>
            {clockField('dayStart', 'Начало дня')}
            {clockField('nightStart', 'Начало ночи')}
            {clockField('horizonEnd', 'Горизонт', 'часы могут быть >24 — это следующий день')}
            {minutesField('Смена партий, мин', turnaroundMin, setTurnaroundMin)}
            {minutesField('Подготовка, мин', prepMin, setPrepMin)}
            <Field label="Режим замеров">
              <select
                className="input"
                value={benchMode}
                onChange={(event) => setBenchMode(event.target.value as BenchMode)}
                disabled={busy}
              >
                <option value="adaptive">Ускоренный (интерполяция)</option>
                <option value="full">Полный (слайс каждой партии)</option>
              </select>
            </Field>
            <AmsCorrectionFields
              available={ams.available}
              on={ams.on}
              onToggle={ams.setOn}
              seconds={ams.seconds}
              onSecondsChange={ams.setSeconds}
              disabled={busy}
            />
          </div>
        </section>
      </form>

      <section className="main-col">
        <div className="action-bar">
          <div className="title">
            <h1>Расписание на сутки</h1>
            <p>{selectedProfile ? selectedProfile.name : 'Принтер определится по проекту'}</p>
          </div>
          <button type="submit" form={FORM_ID} className="btn btn-primary btn-lg" disabled={!canSubmit}>
            {startMutation.isPending ? <span className="spinner" aria-hidden="true" /> : null}
            Построить
          </button>
        </div>
        <JobCard
          jobId={jobId}
          onDismiss={() => setJobId(null)}
          job={job}
          pollError={jobQuery.isError ? jobQuery.error : undefined}
          successContent={
            job?.status === 'SUCCEEDED' && job.schedule ? <ScheduleSummary schedule={job.schedule} /> : null
          }
        />
      </section>
    </div>
  );
}
