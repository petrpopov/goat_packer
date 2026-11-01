import { useEffect, useState } from 'react';
import type { FormEvent } from 'react';
import { useMutation } from '@tanstack/react-query';
import { startCalibrationJob } from '@/api/services';
import AmsCalculator from '@/components/AmsCalculator';
import Field from '@/components/Field';
import FileDrop from '@/components/FileDrop';
import Icon from '@/components/Icon';
import JobCard from '@/components/JobCard';
import { useJobView } from '@/hooks/useJob';
import { useProfileDetection } from '@/hooks/useProfileDetection';
import { useProfiles } from '@/hooks/useProfiles';
import { pushToast } from '@/store/useToastStore';
import { calibrationValues, readStoredCalibration, storeCalibration } from '@/utils/calibration';
import { apiErrorMessage, errorMessage } from '@/utils/jobs';

type Variant = 'normal' | 'perfectionist';

const VARIANTS: { value: Variant; label: string }[] = [
  { value: 'normal', label: 'Обычный — 10 смен, ≈±12 с' },
  { value: 'perfectionist', label: 'Перфекционист — 30 смен, ≈±4 с' },
];

/**
 * «Калибровка AMS»: из своего проекта получить два тестовых кубика (A без смен, B со сменами),
 * напечатать их и по четырём временам посчитать реальную длительность смены прутка.
 * `embedded` — боковая панель упаковки: заголовок с крестиком и поля в одну колонку.
 */
export default function CalibrationPage({ embedded = false, onClose }: { embedded?: boolean; onClose?: () => void }) {
  const [chosenProfileId, setChosenProfileId] = useState('');
  const [variant, setVariant] = useState<Variant>('normal');
  const [file, setFile] = useState<File | null>(null);
  const [jobId, setJobId] = useState<string | null>(null);

  const { profiles, loading: profilesLoading } = useProfiles();

  const detection = useProfileDetection(file);
  const detectedProfileId = detection.data?.profileId ?? null;
  // Принтер файла важнее ручного выбора: сервер всё равно примет только совпадающий профиль.
  const profileId = detectedProfileId ?? chosenProfileId;
  const profile = profiles.find((candidate) => candidate.id === profileId) ?? null;
  const calibrationNeeded = profile === null || profile.filamentChangeSeconds != null;

  const { query: jobQuery, job, active: jobActive } = useJobView(jobId);
  const result = job?.status === 'SUCCEEDED' ? (job.calibration ?? null) : null;

  // Значения теста запоминаются: калькулятор нужен после печати, часто уже без этой задачи.
  useEffect(() => {
    if (result) storeCalibration(result.profileId, calibrationValues(result));
  }, [result]);

  const detectionError = detection.isError
    ? errorMessage(detection.error, 'Не удалось проверить файл')
    : file !== null && detection.isSuccess && detectedProfileId === null
      ? detection.data?.printerModel
        ? `Для принтера «${detection.data.printerModel}» не найден подходящий профиль.`
        : 'В проекте не указан printer_model. Сохраните проект с выбранным принтером в слайсере.'
      : null;

  const startMutation = useMutation({
    mutationFn: startCalibrationJob,
    onSuccess: (response) => {
      setJobId(response.jobId);
      pushToast('Готовлю тестовые файлы', 'success');
    },
    onError: (error) => pushToast(apiErrorMessage(error, 'Не удалось выполнить запрос'), 'error'),
  });

  const busy = jobActive || startMutation.isPending;
  const canSubmit = file !== null && profile !== null && calibrationNeeded && detectedProfileId !== null && !busy;

  const handleSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!canSubmit || !file) return;
    const form = new FormData();
    form.append('file', file);
    form.append('profileId', profileId);
    form.append('variant', variant);
    startMutation.mutate(form);
  };

  const calculatorInitial = result ? calibrationValues(result) : profileId ? readStoredCalibration(profileId) : null;

  return (
    <div className={embedded ? 'stack' : 'page'}>
      {embedded ? (
        <div className="side-head">
          <h2>Калибровка смены прутка</h2>
          <button
            type="button"
            className="icon-btn"
            onClick={onClose}
            aria-label="Закрыть"
            data-tip="Закрыть"
            data-tip-align="end"
          >
            <Icon name="x" />
          </button>
        </div>
      ) : (
        <div>
          <h1 className="page-title">Калибровка AMS</h1>
          <p className="page-subtitle">
            Слайсер занижает время смены прутка. Две короткие печати из вашего проекта покажут, сколько смена длится на
            самом деле — это значение уточнит оценки упаковки и расписания.
          </p>
        </div>
      )}

      <form className="stack" onSubmit={handleSubmit}>
        <section className="card">
          {/* В узкой панели поля идут в одну колонку. */}
          <div className={embedded ? 'stack' : 'form-grid'}>
            <Field
              label="Профиль принтера"
              hint={detectedProfileId ? 'Определён по файлу — выбор заблокирован' : undefined}
            >
              <select
                className="input"
                value={profileId}
                onChange={(event) => setChosenProfileId(event.target.value)}
                disabled={detectedProfileId !== null || busy}
              >
                <option value="">{profilesLoading ? 'Загрузка…' : '— выберите профиль —'}</option>
                {profiles.map((candidate) => (
                  <option key={candidate.id} value={candidate.id}>
                    {candidate.name}
                  </option>
                ))}
              </select>
            </Field>
            <Field label="Вариант теста" hint="больше смен — дольше печать B, но точнее результат">
              <select
                className="input"
                value={variant}
                onChange={(event) => setVariant(event.target.value as Variant)}
                disabled={busy}
              >
                {VARIANTS.map((option) => (
                  <option key={option.value} value={option.value}>
                    {option.label}
                  </option>
                ))}
              </select>
            </Field>
          </div>

          {!calibrationNeeded ? (
            <div className="warn-block">
              Для этого принтера калибровка не нужна: он меняет пруток без AMS, коррекция времени смены ему не
              требуется.
            </div>
          ) : (
            <>
              <p className="field-hint">
                Загрузите любой свой проект 3MF для этого принтера минимум с двумя филаментами: тест возьмёт из него
                принтер, настройки печати, филаменты и тип AMS.
              </p>
              <FileDrop file={file} onFileChange={setFile} disabled={busy} />
              {detectionError ? <div className="error-block">{detectionError}</div> : null}
            </>
          )}

          <div className="form-actions">
            <button type="submit" className="btn btn-primary" disabled={!canSubmit}>
              {startMutation.isPending ? <span className="spinner" aria-hidden="true" /> : null}
              Подготовить тест
            </button>
            {file !== null && detection.isPending ? (
              <span className="field-hint">Определяю принтер по файлу…</span>
            ) : null}
          </div>
        </section>
      </form>

      <JobCard
        jobId={jobId}
        onDismiss={() => setJobId(null)}
        job={job}
        pollError={jobQuery.isError ? jobQuery.error : undefined}
      />

      {calibrationNeeded ? (
        <AmsCalculator
          key={result ? `job-${job?.jobId}` : `profile-${profileId}`}
          profileId={profile?.id ?? null}
          profileName={profile?.name}
          initial={calculatorInitial}
        />
      ) : null}
    </div>
  );
}
