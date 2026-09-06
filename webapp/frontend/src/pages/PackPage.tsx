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
  const [attempts, setAttempts] = useState('2000');
  const [candidates, setCandidates] = useState('1000');
  const [seed, setSeed] = useState('20270831');
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
    form.append('attempts', attempts);
    form.append('candidates', candidates);
    form.append('seed', seed);
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

            <Field label="Чистка сопла">
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
              <Field label="Попытки">
                <input
                  className="input"
                  type="number"
                  min={1}
                  step={1}
                  value={attempts}
                  onChange={(event) => setAttempts(event.target.value)}
                  disabled={jobActive}
                />
              </Field>
              <Field label="Кандидатов на шаг">
                <input
                  className="input"
                  type="number"
                  min={1}
                  step={1}
                  value={candidates}
                  onChange={(event) => setCandidates(event.target.value)}
                  disabled={jobActive}
                />
              </Field>
              <Field label="Seed">
                <input
                  className="input mono"
                  type="number"
                  step={1}
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
