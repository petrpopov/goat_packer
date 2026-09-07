import { useEffect, useMemo, useState } from 'react';
import type { FormEvent } from 'react';
import { useMutation, useQuery } from '@tanstack/react-query';
import { ApiError } from '@/api/http';
import { fetchProfiles, startPackJob } from '@/api/services';
import FileDrop from '@/components/FileDrop';
import Field from '@/components/Field';
import JobCard from '@/components/JobCard';
import { useJob } from '@/hooks/useJob';
import { pushToast } from '@/store/useToastStore';
import { useUiStore } from '@/store/useUiStore';

type NozzleCleanMode = 'off' | 'global' | 'from-z';

/**
 * Уровни усилия поиска: один ползунок вместо двух числовых полей (попытки/кандидаты).
 * «Сильно» — консольный уровень, валидированный на козочках (4800/2400); поиск всё равно
 * останавливается сам, когда долго нет улучшений, так что завышенный потолок почти бесплатен.
 */
const EFFORT_LEVELS = [
  { label: 'Быстро', attempts: 500, candidates: 400 },
  { label: 'Стандарт', attempts: 2000, candidates: 1000 },
  { label: 'Сильно', attempts: 4800, candidates: 2400 },
  { label: 'Максимум', attempts: 12000, candidates: 2400 },
] as const;

/** nozzleClean → "", "global", "global,every=N", "from-z=H", "from-z=H,every=N". */
function buildNozzleClean(mode: NozzleCleanMode, fromZ: string, globalEvery: string, fromZEvery: string): string {
  if (mode === 'off') return '';
  if (mode === 'global') {
    const every = Number.parseInt(globalEvery, 10);
    return Number.isFinite(every) && every > 1 ? `global,every=${every}` : 'global';
  }
  const height = Number.parseFloat(fromZ);
  if (!Number.isFinite(height)) return '';
  const every = Number.parseInt(fromZEvery, 10);
  const base = `from-z=${height}`;
  return Number.isFinite(every) && every > 1 ? `${base},every=${every}` : base;
}

export default function PackPage() {
  const sendToSchedule = useUiStore((state) => state.sendToSchedule);

  const [file, setFile] = useState<File | null>(null);
  const [profileId, setProfileId] = useState('');
  const [clearance, setClearance] = useState('0.7');
  const [resolution, setResolution] = useState('0.25');
  const [effort, setEffort] = useState(EFFORT_LEVELS.length - 2);
  const [seed, setSeed] = useState('');
  const [cleanMode, setCleanMode] = useState<NozzleCleanMode>('off');
  const [globalEvery, setGlobalEvery] = useState('10');
  const [fromZ, setFromZ] = useState('');
  const [fromZEvery, setFromZEvery] = useState('1');
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

  const selectedProfile = profiles.find((profile) => profile.id === profileId) ?? null;
  const nozzleCleanAvailable = selectedProfile === null || selectedProfile.supportsNozzleClean;

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

  const nozzleCleanInvalid = cleanMode === 'from-z' && fromZ.trim() === '';
  const canSubmit = Boolean(file) && profileId !== '' && !jobActive && !startMutation.isPending && !nozzleCleanInvalid;

  const handleSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!file || profileId === '' || nozzleCleanInvalid) return;

    const form = new FormData();
    form.append('file', file);
    form.append('profileId', profileId);
    form.append('clearance', clearance);
    form.append('resolution', resolution);
    form.append('attempts', String(EFFORT_LEVELS[effort].attempts));
    form.append('candidates', String(EFFORT_LEVELS[effort].candidates));
    // Пустой seed = случайный на прогон (бэкенд подставит): повторный запуск может упаковать
    // плотнее — результат сильно зависит от seed. Заполненный = воспроизводимость.
    if (seed.trim() !== '') form.append('seed', seed.trim());
    form.append('nozzleClean', buildNozzleClean(cleanMode, fromZ, globalEvery, fromZEvery));

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
          <FileDrop file={file} onFileChange={setFile} disabled={jobActive} />

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
                  disabled={jobActive}
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
                    disabled={jobActive}
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
                    disabled={jobActive}
                  />
                </Field>
              </>
            )}
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
                  disabled={jobActive}
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
                  disabled={jobActive}
                />
              </Field>
              <Field
                label="Усилие поиска"
                hint={`${EFFORT_LEVELS[effort].attempts} попыток · ${EFFORT_LEVELS[effort].candidates} кандидатов на шаг`}
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
                    disabled={jobActive}
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
              <Field label="Seed" hint="пусто — случайный на прогон; повторный запуск может упаковать плотнее">
                <input
                  className="input mono"
                  type="number"
                  step={1}
                  placeholder="случайный"
                  value={seed}
                  onChange={(event) => setSeed(event.target.value)}
                  disabled={jobActive}
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

      <JobCard job={job} pollError={jobQuery.isError ? jobQuery.error : undefined} successContent={successContent} />
    </div>
  );
}
