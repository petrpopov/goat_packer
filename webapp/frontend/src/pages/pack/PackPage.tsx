import { useCallback, useEffect, useRef, useState } from 'react';
import type { FormEvent } from 'react';
import { useMutation } from '@tanstack/react-query';
import { startPackJob } from '@/api/services';
import Icon from '@/components/Icon';
import { useAmsCorrection } from '@/hooks/useAmsCorrection';
import { useNow } from '@/hooks/useNow';
import { useProfiles } from '@/hooks/useProfiles';
import CalibrationPage from '@/pages/CalibrationPage';
import { pushToast } from '@/store/useToastStore';
import { useUiStore } from '@/store/useUiStore';
import { apiErrorMessage, isActiveStatus, snapmakerTowerNote, towerWarning } from '@/utils/jobs';
import type { CustomBedInput } from '@/utils/printers';
import { DEFAULT_PACK_FORM, EMPTY_BED, packFormData, packRequestParts, valuesFromParams } from './packFormValues';
import type { PackFormValues } from './packFormValues';
import PackForm from './PackForm';
import PackInfoPanel from './PackInfoPanel';
import PackScene from './PackScene';
import { DraftStatusStrip, JobStatusStrip } from './PackStatusStrip';
import { useModelPreview } from './useModelPreview';
import { usePackJob } from './usePackJob';
import { usePrinterSelection } from './usePrinterSelection';

/**
 * Упаковка: черновик (/pack — файл, принтер, параметры) или открытая задача (/pack/<id> — её ход,
 * стол, итог; форма только для чтения). Страница смонтирована всегда: задача переживает переходы.
 */
export default function PackPage() {
  const navigate = useUiStore((state) => state.navigate);
  // Открытая задача — адрес /pack/<id>: при запуске, по ссылке и из списка задач.
  const jobId = useUiStore((state) => state.packJobId);
  const calibOpen = useUiStore((state) => state.calibOpen);
  const setCalibOpen = useUiStore((state) => state.setCalibOpen);
  const draftNonce = useUiStore((state) => state.draftNonce);

  const [file, setFile] = useState<File | null>(null);
  const [values, setValues] = useState<PackFormValues>(DEFAULT_PACK_FORM);
  const update = useCallback(
    <K extends keyof PackFormValues>(key: K, value: PackFormValues[K]) =>
      setValues((current) => ({ ...current, [key]: value })),
    []
  );
  const setBed = useCallback((bed: CustomBedInput) => update('bed', bed), [update]);
  /** Выбор в PrinterPicker (id или CUSTOM_PRINTER); переживает смену файла, определение по 3MF его перекрывает. */
  const [picked, setPicked] = useState<string | null>(null);
  /** Панель «Инфо» поверх сцены: этапы задачи и проверки, ошибка, файлы. */
  const [infoOpen, setInfoOpen] = useState(false);

  // Калибровка нужна только для коррекции внутри окна: без окна панель закрывается.
  useEffect(() => {
    if (!values.windowOn) setCalibOpen(false);
  }, [values.windowOn, setCalibOpen]);

  // «Упаковка» в меню и «Новая упаковка» — чистый черновик: файл и стол прошлой упаковки не переносятся.
  useEffect(() => {
    if (draftNonce === 0) return;
    setFile(null);
    setBed(EMPTY_BED);
  }, [draftNonce, setBed]);

  const { profiles, loading: profilesLoading } = useProfiles(true);
  const selection = usePrinterSelection({ file, picked, profiles, bed: values.bed, setBed });
  const pack = usePackJob(jobId);
  const { job, viewing, external } = pack;
  /** Файл, чья модель на сцене и чьё имя в поле: черновика или открытой задачи. */
  const shownFile = external ? pack.inputFile : file;
  const preview = useModelPreview(shownFile);

  // Задача по ссылке: её параметры — в поля формы (только для чтения; «Упаковать заново» их берёт).
  const appliedFor = useRef<string | null>(null);
  const params = external ? job?.params : null;
  useEffect(() => {
    if (!params || appliedFor.current === jobId) return;
    appliedFor.current = jobId;
    setValues((current) => valuesFromParams(params, current));
  }, [params, jobId]);

  const ams = useAmsCorrection(selection.selectedProfile);
  const parts = packRequestParts(values, {
    nozzleCleanAvailable: selection.selectedProfile === null || selection.selectedProfile.supportsNozzleClean,
    ams,
  });

  const startMutation = useMutation({
    mutationFn: startPackJob,
    onSuccess: (response) => {
      // Строка статуса сама показывает запуск — без тоста; у задачи сразу свой адрес.
      pack.markStartedHere(response.jobId);
      navigate(`/pack/${response.jobId}`);
    },
    onError: (error) => pushToast(apiErrorMessage(error, 'Не удалось выполнить запрос'), 'error'),
  });

  const canSubmit =
    file !== null &&
    // Свой стол: selectedProfile есть, только когда все размеры в границах (parseCustomBed).
    selection.selectedProfile !== null &&
    parts.valid &&
    selection.detection === 'ok' &&
    !viewing &&
    !startMutation.isPending;

  const handleSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!canSubmit || !file) return;
    const target =
      selection.custom && selection.customBed
        ? ({ kind: 'custom', bed: selection.customBed } as const)
        : ({ kind: 'profile', profileId: selection.profileId } as const);
    startMutation.mutate(packFormData(file, values, target, parts));
  };

  /** Новый файл — новая упаковка: открытая задача к нему не относится (идущая продолжает работать). */
  const handleFileChange = (next: File | null) => {
    setFile(next);
    // Размеры своего стола от прошлого файла к новому не относятся: стол проекта нового 3MF заполнит их снова.
    setBed(EMPTY_BED);
    if (viewing) navigate('/pack');
  };
  /** «Упаковать заново»: черновик с теми же файлом и параметрами; у задачи по ссылке файл — её вход. */
  const repack = () => {
    if (external && pack.inputFile) setFile(pack.inputFile);
    navigate('/pack');
  };
  const repackDisabled = external && !pack.inputFile;

  const busy = pack.active || startMutation.isPending;
  const now = useNow(pack.running || pack.checkRunning);
  const closeInfo = useCallback(() => setInfoOpen(false), []);
  /** Принтер сцены: у открытой по ссылке задачи — её собственный (свой стол — тоже). */
  const sceneProfile = external ? (job?.profile ?? null) : selection.selectedProfile;
  /** Готовый стол с башней: GUI слайсера при открытии её сдвинет — над сценой просим вернуть её в угол. */
  const towerNotice = jobId !== null ? towerWarning(job?.status, pack.result) : null;
  /** Snapmaker Orca может увидеть конфликт линий башни и подставки соседней фигурки — пакер из-за этого фигурку не убирает. */
  const conflictNote = jobId !== null ? snapmakerTowerNote(job?.status, pack.result, sceneProfile) : null;

  /** Файл задачи не скачался (истёк срок хранения, нет сети): сцена без модели — сказать прямо. */
  const sceneFailed = external && pack.input.isError ? 'Файл задачи недоступен — стол без модели' : null;
  /** Текст поверх сцены, пока её нечем заполнить. */
  const sceneBusy = sceneFailed
    ? null
    : external && pack.input.isPending && job?.kind === 'pack'
      ? 'Загружаю файл задачи…'
      : preview.parsing
        ? 'Читаю модель…'
        : !viewing && file && selection.detection === 'detecting' && !sceneProfile
          ? 'Определяю принтер…'
          : null;

  return (
    <div className={`work${calibOpen ? ' with-side' : ''}`}>
      <PackForm
        values={values}
        update={update}
        shownFile={shownFile}
        onFileChange={handleFileChange}
        busy={busy}
        locked={viewing || startMutation.isPending || selection.detection === 'detecting'}
        viewing={viewing}
        jobPrinterName={external ? (job?.profile?.name ?? 'Принтер задачи') : undefined}
        parsing={preview.parsing}
        selection={selection}
        profiles={profiles}
        profilesLoading={profilesLoading}
        onPick={setPicked}
        parts={parts}
        ams={ams}
        onCalibrate={() => setCalibOpen(!calibOpen)}
        onSubmit={handleSubmit}
        repack={
          viewing && job && !isActiveStatus(job.status) ? (
            // Параметры задачи слева только для чтения; поменять их — здесь же, новой упаковкой.
            <div className="repack">
              <button type="button" className="btn btn-block" onClick={repack} disabled={repackDisabled}>
                <Icon name="refresh" />
                Упаковать заново
              </button>
              <span className="field-hint">Тот же файл, параметры снова можно менять. Получится новая задача.</span>
            </div>
          ) : null
        }
      />

      <section className="main-col">
        {jobId !== null ? (
          <JobStatusStrip
            jobId={jobId}
            pack={pack}
            fallbackName={shownFile?.name ?? ''}
            now={now}
            infoOpen={infoOpen}
            onToggleInfo={() => setInfoOpen((open) => !open)}
            onOpenInfo={() => setInfoOpen(true)}
            onRepack={repack}
            repackDisabled={repackDisabled}
          />
        ) : (
          <DraftStatusStrip
            file={file}
            selection={selection}
            parsing={preview.parsing}
            canSubmit={canSubmit}
            submitting={startMutation.isPending}
          />
        )}
        {towerNotice ? (
          <div className="warn-block tower-notice" role="note">
            <Icon name="alert" />
            {towerNotice}
          </div>
        ) : null}
        {conflictNote ? (
          <div className="warn-block tower-notice" role="note">
            <Icon name="alert" />
            {conflictNote}
          </div>
        ) : null}
        <PackScene
          profile={sceneProfile}
          model={preview.model}
          modelTooLarge={preview.tooLarge}
          pack={pack}
          busyText={sceneBusy}
          failedText={sceneFailed}
          overlay={
            infoOpen && jobId !== null && job ? (
              <PackInfoPanel jobId={jobId} job={job} pack={pack} now={now} onClose={closeInfo} />
            ) : null
          }
        />
      </section>

      {/* Скрыта, а не размонтирована: jobId калибровки живёт в её состоянии и терялся при закрытии. */}
      <aside className="side-col" hidden={!calibOpen}>
        <CalibrationPage embedded onClose={() => setCalibOpen(false)} />
      </aside>
    </div>
  );
}
