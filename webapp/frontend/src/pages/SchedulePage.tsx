import { useEffect, useMemo, useState } from 'react';
import type { FormEvent } from 'react';
import { useMutation, useQuery } from '@tanstack/react-query';
import { ApiError } from '@/api/http';
import { fetchProfiles, startScheduleJob } from '@/api/services';
import FileDrop from '@/components/FileDrop';
import Field from '@/components/Field';
import JobCard from '@/components/JobCard';
import { useJob } from '@/hooks/useJob';
import { pushToast } from '@/store/useToastStore';
import { useUiStore } from '@/store/useUiStore';
import { formatGrams, formatMinutes } from '@/utils/format';

type SourceKind = 'pack' | 'file';
type BenchMode = 'adaptive' | 'full';

/** Часы:Минуты; часы могут быть > 24 (следующий день), поэтому 00–99. */
const CLOCK_RE = /^\d{1,2}:[0-5]\d$/;

function isClock(value: string): boolean {
  return CLOCK_RE.test(value.trim());
}

export default function SchedulePage() {
  const packHandoff = useUiStore((state) => state.packHandoff);

  const [source, setSource] = useState<SourceKind>(packHandoff ? 'pack' : 'file');
  const [file, setFile] = useState<File | null>(null);
  const [profileId, setProfileId] = useState(packHandoff?.profileId ?? '');
  const [dayStart, setDayStart] = useState('10:00');
  const [nightStart, setNightStart] = useState('23:00');
  const [horizonEnd, setHorizonEnd] = useState('34:00');
  const [turnaroundMin, setTurnaroundMin] = useState('10');
  const [prepMin, setPrepMin] = useState('10');
  const [benchMode, setBenchMode] = useState<BenchMode>('adaptive');
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

  const jobQuery = useJob(jobId);
  const job = jobQuery.data ?? null;
  const jobActive = job?.status === 'QUEUED' || job?.status === 'RUNNING';

  const dayStartValid = isClock(dayStart);
  const nightStartValid = isClock(nightStart);
  const horizonEndValid = isClock(horizonEnd);
  const timesValid = dayStartValid && nightStartValid && horizonEndValid;

  const startMutation = useMutation({
    mutationFn: startScheduleJob,
    onSuccess: (response) => {
      setJobId(response.jobId);
      pushToast('Задача расписания запущена', 'success');
    },
    onError: (error) => {
      pushToast(error instanceof ApiError ? error.message : 'Не удалось выполнить запрос', 'error');
    },
  });

  const sourceReady = source === 'pack' ? packHandoff !== null : file !== null;
  const canSubmit = sourceReady && profileId !== '' && timesValid && !jobActive && !startMutation.isPending;

  const handleSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!timesValid || profileId === '') return;
    if (source === 'pack' ? !packHandoff : !file) return;

    const form = new FormData();
    if (source === 'pack' && packHandoff) {
      form.append('packJobId', packHandoff.packJobId);
    } else if (file) {
      form.append('file', file);
    }
    form.append('profileId', profileId);
    form.append('dayStart', dayStart.trim());
    form.append('nightStart', nightStart.trim());
    form.append('horizonEnd', horizonEnd.trim());
    form.append('turnaroundMin', turnaroundMin);
    form.append('prepMin', prepMin);
    form.append('benchMode', benchMode);

    startMutation.mutate(form);
  };

  const schedule = job?.status === 'SUCCEEDED' ? job.schedule : null;

  const successContent = schedule ? (
    <div className="stack-sm">
      <p className="section-caption">Сводка</p>
      <div className="kv panel-2">
        <div className="kv-item">
          <span className="kv-label">Принтер</span>
          <span className="kv-value">{schedule.printer}</span>
        </div>
        <div className="kv-item">
          <span className="kv-label">Всего объектов</span>
          <span className="kv-value mono">{schedule.totalObjects}</span>
        </div>
        <div className="kv-item">
          <span className="kv-label">Филамент</span>
          <span className="kv-value mono">{formatGrams(schedule.totalFilamentGrams)} г</span>
        </div>
        <div className="kv-item">
          <span className="kv-label">Завершение</span>
          <span className="kv-value mono">{schedule.completionClock}</span>
        </div>
      </div>

      <p className="section-caption">Работы</p>
      <div className="table-wrap">
        <table className="table table-compact">
          <thead>
            <tr>
              <th className="num">№</th>
              <th className="num">Партия</th>
              <th>Начало</th>
              <th>Конец</th>
              <th className="num">Слайс, мин</th>
              <th className="num">Подготовка, мин</th>
              <th className="num">Филамент, г</th>
              <th className="num">Накопительно</th>
              <th>Ночь</th>
            </tr>
          </thead>
          <tbody>
            {schedule.jobs.map((entry) => (
              <tr key={entry.sequence}>
                <td className="num mono">{entry.sequence}</td>
                <td className="num mono">{entry.batchSize}</td>
                <td className="mono">{entry.startClock}</td>
                <td className="mono">{entry.endClock}</td>
                <td className="num mono">{formatMinutes(entry.slicerSeconds)}</td>
                <td className="num mono">{formatMinutes(entry.preparationSeconds)}</td>
                <td className="num mono">{formatGrams(entry.filamentGrams)}</td>
                <td className="num mono">{entry.cumulativeObjects}</td>
                <td>
                  {entry.overnight ? (
                    <span className="pill pill-warn">да</span>
                  ) : (
                    <span className="pill pill-muted">нет</span>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      {schedule.plates.length > 0 && (
        <>
          <p className="section-caption">Листы</p>
          <div className="pill-row">
            {schedule.plates.map((plate) => (
              <span className="pill pill-cyan" key={plate.label}>
                {plate.label}
              </span>
            ))}
          </div>
        </>
      )}
    </div>
  ) : null;

  return (
    <div className="page">
      <div>
        <h1 className="page-title">Фаза 2 · Расписание</h1>
        <p className="page-subtitle">Планирование партий печати по окнам дня и ночи</p>
      </div>

      <form className="stack" onSubmit={handleSubmit}>
        <section className="card">
          <div className="source-switch">
            {packHandoff && (
              <button
                type="button"
                className={`btn${source === 'pack' ? ' btn-primary' : ''}`}
                onClick={() => setSource('pack')}
              >
                Из фазы 1
              </button>
            )}
            <button
              type="button"
              className={`btn${source === 'file' ? ' btn-primary' : ''}`}
              onClick={() => setSource('file')}
            >
              Файл .3mf
            </button>
          </div>

          {source === 'pack' && packHandoff ? (
            <p className="source-note">
              Артефакт фазы 1: job <span className="mono">{packHandoff.packJobId}</span>
              <button type="button" className="btn-ghost" onClick={() => setSource('file')}>
                Заменить файлом
              </button>
            </p>
          ) : (
            <FileDrop file={file} onFileChange={setFile} disabled={jobActive} />
          )}

          <div className="form-grid">
            <Field label="Профиль принтера">
              <select
                className="input"
                value={profileId}
                onChange={(event) => setProfileId(event.target.value)}
                disabled={profilesQuery.isPending || jobActive}
              >
                <option value="">{profilesQuery.isPending ? 'Загрузка…' : '— выберите профиль —'}</option>
                {profiles.map((profile) => (
                  <option key={profile.id} value={profile.id}>
                    {profile.name} · {profile.bedWidthMm}×{profile.bedDepthMm} мм
                  </option>
                ))}
              </select>
            </Field>

            <Field label="Начало дня" error={!dayStartValid} hint={dayStartValid ? undefined : 'Формат ЧЧ:ММ'}>
              <input
                className="input mono"
                type="text"
                placeholder="10:00"
                value={dayStart}
                onChange={(event) => setDayStart(event.target.value)}
                disabled={jobActive}
              />
            </Field>

            <Field label="Начало ночи" error={!nightStartValid} hint={nightStartValid ? undefined : 'Формат ЧЧ:ММ'}>
              <input
                className="input mono"
                type="text"
                placeholder="23:00"
                value={nightStart}
                onChange={(event) => setNightStart(event.target.value)}
                disabled={jobActive}
              />
            </Field>

            <Field
              label="Горизонт"
              error={!horizonEndValid}
              hint={!horizonEndValid ? 'Формат ЧЧ:ММ' : 'часы могут быть >24 — это следующий день'}
            >
              <input
                className="input mono"
                type="text"
                placeholder="34:00"
                value={horizonEnd}
                onChange={(event) => setHorizonEnd(event.target.value)}
                disabled={jobActive}
              />
            </Field>

            <Field label="Смена партий, мин">
              <input
                className="input"
                type="number"
                min={0}
                step={1}
                value={turnaroundMin}
                onChange={(event) => setTurnaroundMin(event.target.value)}
                disabled={jobActive}
              />
            </Field>

            <Field label="Подготовка, мин">
              <input
                className="input"
                type="number"
                min={0}
                step={1}
                value={prepMin}
                onChange={(event) => setPrepMin(event.target.value)}
                disabled={jobActive}
              />
            </Field>

            <Field label="Режим замеров">
              <select
                className="input"
                value={benchMode}
                onChange={(event) => setBenchMode(event.target.value as BenchMode)}
                disabled={jobActive}
              >
                <option value="adaptive">Ускоренный (интерполяция)</option>
                <option value="full">Полный (слайс каждой партии)</option>
              </select>
            </Field>
          </div>

          <div className="form-actions">
            <button type="submit" className="btn btn-primary" disabled={!canSubmit}>
              {startMutation.isPending ? <span className="spinner" aria-hidden="true" /> : null}
              Построить расписание
            </button>
          </div>
        </section>
      </form>

      <JobCard job={job} pollError={jobQuery.isError ? jobQuery.error : undefined} successContent={successContent} />
    </div>
  );
}
