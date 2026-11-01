import { lazy, Suspense, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { FormEvent, ReactNode } from 'react';
import { useMutation, useQuery } from '@tanstack/react-query';
import { ApiError } from '@/api/http';
import {
  cancelPackCheck,
  checkPackJob,
  deleteJob,
  downloadJobInput,
  fetchProfiles,
  jobLink,
  startPackJob,
} from '@/api/services';
import type { JobArtifact, JobStage, PackJobInfo } from '@/api/types';
import AmsCorrectionFields from '@/components/AmsCorrectionFields';
import ErrorBoundary from '@/components/ErrorBoundary';
import FileDrop from '@/components/FileDrop';
import Field from '@/components/Field';
import Icon from '@/components/Icon';
import JobFiles, { startDownload, visibleArtifacts } from '@/components/JobFiles';
import ScenePopover from '@/components/ScenePopover';
import StatusStrip, { type StripLine } from '@/components/StatusStrip';
import PrinterPicker, { CUSTOM_PRINTER } from '@/components/PrinterPicker';
import CalibrationPage from '@/pages/CalibrationPage';
import { sceneLayout } from '@/scene/layout';
import type { ModelMesh } from '@/scene/model3d';
import { parseModelInWorker, PREVIEW_MAX_BYTES } from '@/scene/parseModelAsync';
import { buildNozzleClean, buildSupportOverlap, numberInRange, packTimeFields, parseNozzleClean } from '@/utils/forms';
import { formatBytes, formatHoursMinutes } from '@/utils/format';
import {
  formatDuration,
  objectsText,
  sceneClip,
  stageClockText,
  stageElapsed,
  stageRows,
  stripStage,
} from '@/utils/progress';
import {
  customBedFromProject,
  customPrinterProfile,
  isStlName,
  parseCustomBed,
  type CustomBedInput,
} from '@/utils/printers';
import { useAmsCorrection } from '@/hooks/useAmsCorrection';
import { useProfileDetection } from '@/hooks/useProfileDetection';
import { useJob } from '@/hooks/useJob';
import { pushToast } from '@/store/useToastStore';
import { useUiStore } from '@/store/useUiStore';

type NozzleCleanMode = 'off' | 'global' | 'from-z';

/**
 * Принтер задачи: определён по 3MF (выбор заблокирован), выбран пользователем (STL или до файла),
 * свой стол («Моего принтера нет в списке» или 3MF неизвестного принтера — размеры из проекта).
 */
type PrinterChoice = { kind: 'detected'; id: string } | { kind: 'picked'; id: string } | { kind: 'custom' } | null;

const EMPTY_BED: CustomBedInput = { width: '', depth: '', height: '', margin: '5' };

// three.js тяжёлый: сцена грузится отдельным чанком, только когда известен профиль.
// Одна повторная попытка: сетевой сбой при загрузке чанка иначе сразу уводит в заглушку.
const BedScene = lazy(() => import('@/scene/BedScene').catch(() => import('@/scene/BedScene')));

const ACTIVE = ['QUEUED', 'RUNNING', 'FINALIZING', 'LOST', 'RETRY_WAIT'];

/** Разделитель «заголовок / сырые детали» ошибки (CliErrorTranslator бэкенда). */
const DETAILS_SEPARATOR = '\n— детали —\n';

/** Секундомер этапа между опросами: тикает раз в секунду, только пока идёт задача. */
function useNow(active: boolean): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active) return;
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [active]);
  return now;
}

const Num = ({ children }: { children: ReactNode }) => <span className="num">{children}</span>;

export default function PackPage() {
  const navigate = useUiStore((state) => state.navigate);
  // Открытая задача — адрес /pack/<id>: при запуске, по ссылке и из списка задач.
  const jobId = useUiStore((state) => state.packJobId);
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
  /** Сразу нарезать весь стол; иначе — по кнопке «Нарезать стол» после упаковки (минуты ожидания). */
  const [sliceNow, setSliceNow] = useState(false);
  const [windowHours, setWindowHours] = useState('12');
  const [marginPct, setMarginPct] = useState('10');
  const [supportOverlap, setSupportOverlap] = useState('');
  const [model, setModel] = useState<ModelMesh | null>(null);
  /** Разбор модели в воркере идёт — на столе «Читаю модель…». */
  const [parsing, setParsing] = useState(false);
  /** Задачи, запущенные с этой вкладки: их модель — уже разобранный файл формы, качать вход не нужно. */
  const [startedHere, setStartedHere] = useState<ReadonlySet<string>>(() => new Set());
  /** Панель «Инфо» поверх сцены: этапы задачи и проверки, ошибка, файлы. */
  const [infoOpen, setInfoOpen] = useState(false);
  /** Выбор в PrinterPicker (id или CUSTOM_PRINTER); переживает смену файла, определение по 3MF его перекрывает. */
  const [picked, setPicked] = useState<string | null>(null);
  const [bedInput, setBedInput] = useState<CustomBedInput>(EMPTY_BED);

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
  const detected = detectMutation.data;
  const stl = file !== null && isStlName(file.name);
  const detectedId = !stl ? (detected?.profileId ?? null) : null;
  // 3MF неизвестного принтера: пакер примет его только со своим столом (сверяет printer_model с профилем).
  const unknown3mf = file !== null && !stl && detectMutation.isSuccess && detectedId === null;
  const projectBed = unknown3mf ? (detected?.projectBed ?? null) : null;
  const choice: PrinterChoice = detectedId
    ? { kind: 'detected', id: detectedId }
    : unknown3mf
      ? { kind: 'custom' }
      : file !== null && !stl
        ? null // 3MF ещё определяется или не прочитался
        : picked === CUSTOM_PRINTER
          ? { kind: 'custom' }
          : picked
            ? { kind: 'picked', id: picked }
            : null;
  const profileId = choice && choice.kind !== 'custom' ? choice.id : '';

  // Стол из проекта неизвестного принтера — в поля своего стола: размеры только для показа (слайсер
  // режет проект по своему столу, сервер правку отвергнет), отступ — свой.
  useEffect(() => {
    if (projectBed) setBedInput(customBedFromProject(projectBed));
  }, [projectBed]);

  const customBed = parseCustomBed(bedInput);
  const customKey = customBed ? `${customBed.width}|${customBed.depth}|${customBed.height}|${customBed.margin}` : '';
  // Сцена перестраивает стол при смене объекта профиля: синтетический профиль — только при смене размеров.
  const customProfile = useMemo(
    () => (customBed ? customPrinterProfile(customBed, projectBed) : null),
    // customBed пересоздаётся на каждом рендере; его значения — в customKey.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [customKey, projectBed]
  );
  const jobQuery = useJob(jobId);
  const job = jobQuery.data ?? null;
  const jobGone = jobQuery.error instanceof ApiError && [404, 410].includes(jobQuery.error.status);
  const jobActive = Boolean(jobId) && !jobGone && (!job || ACTIVE.includes(job.status));
  /** Открыта задача (её параметры — только для чтения, «Упаковать заново» возвращает к черновику). */
  const viewing = jobId !== null;
  /** Задача открыта по ссылке или из списка: модель и параметры — с сервера, не из формы. */
  const external = viewing && !startedHere.has(jobId);
  const jobInput = useQuery({
    queryKey: ['job-input', jobId],
    enabled: external && job?.kind === 'pack' && Boolean(job.originalName),
    queryFn: ({ signal }) => downloadJobInput(jobId!, job!.originalName!, signal),
    staleTime: Infinity,
    retry: false,
  });
  const jobFile = external ? (jobInput.data ?? null) : null;
  /** Файл, чья модель на сцене и чьё имя в поле: черновика или открытой задачи. */
  const shownFile = external ? jobFile : file;

  // Предпросмотр модели в сцене разбирается в браузере из того же файла, что уйдёт на сервер —
  // в воркере, чтобы разбор крупного 3MF не замораживал страницу.
  useEffect(() => {
    let cancelled = false;
    let cancelParse: (() => void) | null = null;
    setModel(null);
    setParsing(false);
    if (!shownFile || shownFile.size > PREVIEW_MAX_BYTES) return;
    setParsing(true);
    shownFile
      .arrayBuffer()
      .then((buffer) => {
        if (cancelled) return null;
        const parse = parseModelInWorker(shownFile.name, buffer);
        cancelParse = parse.cancel;
        return parse.promise;
      })
      .then((mesh) => {
        if (!cancelled && mesh) setModel(mesh);
      })
      .catch((error: unknown) => {
        if (!cancelled) pushToast(error instanceof Error ? error.message : 'Не удалось прочитать модель', 'error');
      })
      .finally(() => {
        if (!cancelled) setParsing(false);
      });
    return () => {
      cancelled = true;
      cancelParse?.();
    };
  }, [shownFile]);

  // Задача по ссылке: её параметры — в поля формы (только для чтения; «Упаковать заново» их берёт).
  const appliedFor = useRef<string | null>(null);
  const params = external ? job?.params : null;
  useEffect(() => {
    if (!params || appliedFor.current === jobId) return;
    appliedFor.current = jobId;
    setClearance(String(params.clearance));
    setResolution(String(params.resolution));
    setLayout(params.layout === 'grid' ? 'grid' : 'search');
    const clean = parseNozzleClean(params.nozzleClean);
    setCleanMode(clean.mode);
    if (clean.mode === 'global') setGlobalEvery(clean.every);
    if (clean.mode === 'from-z') {
      setFromZ(clean.fromZ);
      setFromZEvery(clean.every);
    }
    setEdgeMargin(params.edgeMargin == null ? '' : String(params.edgeMargin));
    setSliceNow(params.finalSlice);
    setWindowOn(params.timeWindowHours != null);
    if (params.timeWindowHours != null) setWindowHours(String(params.timeWindowHours));
    if (params.timeMarginPct != null) setMarginPct(String(params.timeMarginPct));
    setSupportOverlap(params.supportOverlapMm == null ? '' : String(params.supportOverlapMm));
    if (params.customBed)
      setBedInput({
        width: String(params.customBed.width),
        depth: String(params.customBed.depth),
        height: String(params.customBed.height),
        margin: String(params.customBed.margin),
      });
  }, [params, jobId]);

  /** Новый файл — новая упаковка: открытая задача к нему не относится (идущая продолжает работать). */
  const handleFileChange = (next: File | null) => {
    setFile(next);
    // Размеры своего стола от прошлого файла к новому не относятся: стол проекта нового 3MF заполнит их снова.
    setBedInput(EMPTY_BED);
    if (viewing) navigate('/pack');
  };

  const selectedProfile =
    choice?.kind === 'custom' ? customProfile : (profiles.find((profile) => profile.id === profileId) ?? null);
  const nozzleCleanAvailable = selectedProfile === null || selectedProfile.supportsNozzleClean;
  const ams = useAmsCorrection(selectedProfile);

  // Состояние файла: idle (нет) / detecting (загрузка и определение) / ok (можно паковать) / unknown
  // (3MF без printer_model — его не принять даже со своим столом) / error.
  const detection =
    file === null
      ? 'idle'
      : detectMutation.isPending
        ? 'detecting'
        : detectMutation.isError
          ? 'error'
          : unknown3mf && !detected?.printerModel
            ? 'unknown'
            : 'ok';
  const detectionError = detectMutation.isError
    ? detectMutation.error instanceof Error
      ? detectMutation.error.message
      : 'Не удалось проверить файл'
    : detection === 'unknown'
      ? 'В проекте не указан printer_model. Сохраните проект с выбранным принтером в слайсере.'
      : null;
  const pickerHint =
    file !== null && !stl && detection === 'detecting'
      ? 'Определяю принтер по файлу…'
      : unknown3mf
        ? projectBed
          ? 'Принтер не в каталоге — размеры взяты из проекта'
          : `Принтер${detected?.printerModel ? ` «${detected.printerModel}»` : ''} не в каталоге — укажите размеры стола`
        : stl
          ? 'STL: выберите принтер — проект соберём на сервере'
          : file === null
            ? 'можно выбрать заранее — стол появится справа'
            : undefined;
  const custom = choice?.kind === 'custom';

  const startMutation = useMutation({
    mutationFn: startPackJob,
    onSuccess: (response) => {
      // Строка статуса сама показывает запуск — без тоста; у задачи сразу свой адрес.
      setStartedHere((ids) => new Set(ids).add(response.jobId));
      navigate(`/pack/${response.jobId}`);
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
    // Свой стол: selectedProfile есть, только когда все размеры в границах (parseCustomBed).
    selectedProfile !== null &&
    numbersValid &&
    detection === 'ok' &&
    !viewing &&
    !startMutation.isPending &&
    !nozzleCleanInvalid &&
    !timeInvalid &&
    !supportInvalid;

  const handleSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!canSubmit || !file) return;

    const form = new FormData();
    form.append('file', file);
    if (custom && customBed) {
      form.append('customWidth', String(customBed.width));
      form.append('customDepth', String(customBed.depth));
      form.append('customHeight', String(customBed.height));
      form.append('customMargin', String(customBed.margin));
    } else {
      form.append('profileId', profileId);
    }
    form.append('clearance', clearance);
    form.append('resolution', resolution);
    // attempts не шлём: действует дефолт бэкенда (= CLI). Ползунок усилия убран — с поддержками
    // поиск всегда перебирает фиксированные 64 варианта, число попыток ни на что не влияло.
    form.append('nozzleClean', nozzleSpec);
    form.append('layout', layout);
    // Пустой отступ = дефолт профиля (у Snapmaker U1 — 4 мм против качания головки). Свой стол
    // несёт отступ в customMargin, второй (из «Дополнительно», там скрыт) не шлём.
    if (!custom && edgeMargin.trim() !== '') form.append('edgeMargin', edgeMargin.trim());
    // Окно и коррекция — только если заданы: иначе действуют дефолты CLI.
    for (const [name, value] of timeFields) form.append(name, value);
    // Пустое перекрытие = без ограничений: поле не шлётся, флаг CLI не передаётся.
    for (const [name, value] of supportFields) form.append(name, value);
    // Окно печати меряется нарезками — с ним стол режется всегда.
    form.append('finalSlice', String(sliceNow || windowOn));

    startMutation.mutate(form);
  };

  // Проверка нарезкой идёт внутри той же задачи: адрес не меняется, опрос подхватывает её ход.
  const checkMutation = useMutation({
    mutationFn: checkPackJob,
    onSuccess: () => void jobQuery.refetch(),
    onError: (error) => {
      pushToast(error instanceof ApiError ? error.message : 'Не удалось запустить проверку', 'error');
    },
  });
  const cancelCheckMutation = useMutation({
    mutationFn: cancelPackCheck,
    onSuccess: () => void jobQuery.refetch(),
    onError: (error) =>
      pushToast(error instanceof ApiError ? error.message : 'Не удалось остановить проверку', 'error'),
  });
  const cancelMutation = useMutation({
    mutationFn: (id: string) => deleteJob(id),
    onSuccess: () => navigate('/pack'),
    onError: (error) => {
      if (error instanceof ApiError && error.status === 404) navigate('/pack');
      else pushToast(error.message, 'error');
    },
  });

  const busy = jobActive || startMutation.isPending;
  /** Форма только для чтения: открыта задача, идёт отправка или файл ещё загружается. */
  const formLocked = viewing || startMutation.isPending || detection === 'detecting';
  /** Принтер сцены: у открытой по ссылке задачи — её собственный (свой стол — тоже). */
  const viewProfile = external ? (job?.profile ?? null) : selectedProfile;
  const running = job?.status === 'RUNNING' || job?.status === 'FINALIZING';
  // Ссылка на layout стабильна между опросами (structural sharing react-query) — сцена не перестраивается зря.
  const jobLayout = job?.layout;
  const placed = useMemo(() => sceneLayout(jobLayout), [jobLayout]);
  // Проверка нарезкой идёт внутри готовой задачи: стол на это время — в режиме нарезки (срез слоя).
  const checkSlicing = job?.check?.status === 'RUNNING' || job?.check?.status === 'FINALIZING';
  const sceneMode = running || checkSlicing ? 'running' : job?.status === 'SUCCEEDED' ? 'done' : 'idle';
  /** Идёт перебор раскладок, позиций ещё нет: на столе мелькают проекции модели. */
  const searching = running && job?.stage?.id === 'packing' && placed.placements.length === 0;

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
      <Field
        label="Зазор, мм"
        hint="наименьшее расстояние между соседями вместе с их поддержками и каймой; меньше — плотнее, но поддержки могут слипнуться"
      >
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
      <Field
        label="Растр, мм"
        hint="шаг сетки поиска раскладки; меньше — точнее и дольше, на плотность обычно не влияет"
      >
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
      {custom ? null : (
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
      )}
    </>
  );

  const bedComplete = (['width', 'depth', 'height'] as const).every((key) => {
    const value = Number(bedInput[key]);
    return bedInput[key].trim() !== '' && value >= 50 && value <= 1000;
  });
  // Размеры из стола проекта не правятся (высота — если она в проекте есть).
  const fixedByProject = (key: keyof CustomBedInput) =>
    projectBed !== null &&
    (key === 'width' || key === 'depth' || (key === 'height' && projectBed.printableHeight != null));
  const bedField = (key: keyof CustomBedInput, label: string, min: number, max: number, hint?: string) => {
    const raw = bedInput[key];
    const fixed = fixedByProject(key);
    const value = Number(raw);
    // Пустое поле не краснеет (его ещё не заполнили); кнопка «Упаковать» без всех размеров неактивна.
    const outOfRange = raw.trim() !== '' && !(Number.isFinite(value) && value >= min && value <= max);
    // Все поля в границах, а стол не собирается — значит, отступ съедает его (как CustomBed.validate).
    const marginTooLarge = key === 'margin' && !outOfRange && !customBed && bedComplete;
    const invalid = outOfRange || marginTooLarge;
    return (
      <Field
        label={label}
        error={invalid}
        hint={
          outOfRange
            ? `${min}–${max} мм`
            : marginTooLarge
              ? 'отступ больше половины стола'
              : fixed
                ? 'из проекта'
                : hint
        }
      >
        <input
          className="input"
          type="number"
          min={min}
          max={max}
          step="any"
          inputMode="decimal"
          placeholder={key === 'margin' ? '5' : undefined}
          value={raw}
          onChange={(event) => setBedInput((bed) => ({ ...bed, [key]: event.target.value }))}
          readOnly={fixed}
          disabled={busy}
        />
      </Field>
    );
  };
  const customBedFields = custom ? (
    <div className="nested custom-bed">
      <div className="row3">
        {bedField('width', 'Ширина, мм', 50, 1000)}
        {bedField('depth', 'Глубина, мм', 50, 1000)}
        {bedField('height', 'Высота, мм', 50, 1000)}
      </div>
      {bedField('margin', 'Отступ от края, мм', 0, 30, 'линия прочистки неизвестна — держим запас')}
    </div>
  ) : null;

  // ───── Строка статуса: имя файла и номер задачи (не меняются), что происходит, действия ─────
  const check = job?.check ?? null;
  const checkActive = check !== null && ACTIVE.includes(check.status);
  const checkRunning = check?.status === 'RUNNING' || check?.status === 'FINALIZING';
  const checkDone = check?.status === 'SUCCEEDED';
  const now = useNow(running || checkRunning);
  const nowAfter = (receivedAtMs: number | undefined) => Math.max(now, receivedAtMs ?? 0);
  const copyLink = useCallback(() => {
    if (!jobId) return;
    navigator.clipboard
      .writeText(jobLink(jobId))
      .then(() => pushToast('Ссылка скопирована', 'success'))
      .catch(() => pushToast('Не удалось скопировать ссылку', 'error'));
  }, [jobId]);
  const closeInfo = useCallback(() => setInfoOpen(false), []);
  const newDraft = useUiStore((state) => state.newDraft);
  const draftNonce = useUiStore((state) => state.draftNonce);
  // «Упаковка» в меню и «Новая упаковка» — чистый черновик: файл и стол прошлой упаковки не переносятся.
  useEffect(() => {
    if (draftNonce === 0) return;
    setFile(null);
    setBedInput(EMPTY_BED);
  }, [draftNonce]);
  /** «Упаковать заново»: черновик с теми же файлом и параметрами; у задачи по ссылке файл — её вход. */
  const repack = () => {
    if (external && jobFile) setFile(jobFile);
    navigate('/pack');
  };
  // Тихая иконка слева от кнопок: ссылкой делятся реже, чем скачивают.
  const copyButton = jobId ? (
    <button
      type="button"
      className="icon-btn copy-quiet"
      aria-label="Скопировать ссылку на задачу"
      data-tip="Скопировать ссылку на задачу"
      data-tip-align="end"
      onClick={copyLink}
    >
      <Icon name="copy" />
    </button>
  ) : null;
  /** Итог для скачивания: после проверки нарезкой — её стол (она могла сдвинуть объекты). */
  const result: PackJobInfo | null = checkDone && check?.pack ? check.pack : (job?.pack ?? null);
  const deliverables: JobArtifact[] = checkDone && check ? check.artifacts : (job?.artifacts ?? []);
  const packed3mf = visibleArtifacts(deliverables).find((a) => a.name.endsWith('.3mf'));
  const uploadText = detectMutation.upload ? (
    <>
      Загрузка файла ·{' '}
      <Num>{Math.round((detectMutation.upload.sent / Math.max(1, detectMutation.upload.total)) * 100)}</Num> %
    </>
  ) : null;
  /** Время до конца этапа или сколько он уже идёт (если у пакера нет оценки). */
  const stageClock = (stage: JobStage, receivedAtMs: number | undefined) => {
    return stageClockText(stageElapsed(stage, receivedAtMs, nowAfter(receivedAtMs)), stage.expectedSeconds);
  };
  const queueText = (position: number | null | undefined) =>
    position != null ? (
      <>
        место <Num>{position}</Num>
        {job?.estimatedWaitFromSeconds != null && job.estimatedWaitToSeconds != null && job.status === 'QUEUED' ? (
          <>
            {' '}
            · начнётся через ≈
            <Num>
              {Math.floor(job.estimatedWaitFromSeconds / 60)}–{Math.ceil(job.estimatedWaitToSeconds / 60)}
            </Num>{' '}
            мин
          </>
        ) : null}
      </>
    ) : (
      'ждёт свободного исполнителя'
    );
  const joinParts = (parts: ReactNode[]) =>
    parts.map((part, index) => (
      <span key={index}>
        {index ? ' · ' : ''}
        {part}
      </span>
    ));

  let title: ReactNode;
  let detail: ReactNode;
  let line: StripLine = { kind: 'off' };
  let lineLabel: string | undefined;
  let actions: ReactNode;
  const submitButton = (label: string, pending = false) => (
    <button
      type="submit"
      form="pack-form"
      className="btn btn-primary btn-lg btn-main"
      disabled={!canSubmit}
      data-tip={canSubmit ? 'Разложить объекты на столе' : undefined}
      data-tip-align="end"
    >
      {pending ? <span className="spinner" aria-hidden="true" /> : null}
      {label}
    </button>
  );
  const jobName = job?.originalName ?? shownFile?.name ?? '';
  // Рядом с именем — сколько объектов получилось (id задачи пользователю ничего не говорит); без итога — ничего.
  const resultCount = job?.status === 'SUCCEEDED' && result ? result.objectCount : null;
  const jobTitle = jobId ? (
    <>
      <span className="fname">{jobName || 'Задача'}</span>
      {resultCount != null ? <span className="job-id">{objectsText(resultCount)}</span> : null}
    </>
  ) : null;

  if (viewing && jobGone) {
    title = jobTitle;
    detail = 'Задача недоступна: удалена или истёк срок хранения файлов';
    actions = (
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
    );
  } else if (viewing) {
    const status = job?.status;
    title = jobTitle;
    const cancel = (
      <button
        type="button"
        className="btn btn-lg btn-main btn-cancel"
        disabled={cancelMutation.isPending || job?.canManage === false}
        onClick={() => jobId && cancelMutation.mutate(jobId)}
        data-tip="Остановить и удалить задачу"
        data-tip-align="end"
      >
        <Icon name="stop" />
        Отменить
      </button>
    );
    if (!job) {
      detail = jobQuery.isError && jobQuery.error instanceof Error ? jobQuery.error.message : 'Получаю статус задачи…';
      line = { kind: 'waiting' };
      actions = copyButton;
    } else if (status === 'QUEUED' || status === 'RETRY_WAIT' || status === 'LOST') {
      // Повтор — с причиной от сервера («Сервер перезапускался — задача запущена заново»).
      detail = joinParts([
        status === 'QUEUED' ? 'В очереди' : (job.phaseMessage ?? 'Задача запущена заново'),
        queueText(job.queuePosition),
      ]);
      line = { kind: 'waiting' };
      actions = (
        <>
          {copyButton}
          {cancel}
        </>
      );
    } else if (status === 'RUNNING' || status === 'FINALIZING') {
      const stage = job.stage;
      if (stage) {
        const strip = stripStage(stage);
        detail = joinParts([
          <>
            Упаковка · этап <Num>{strip.step}</Num> из <Num>{strip.steps}</Num>
          </>,
          strip.label.charAt(0).toLowerCase() + strip.label.slice(1),
          stageClock(stage, job.receivedAtMs),
        ]);
        lineLabel = `${strip.label}, этап ${strip.step} из ${strip.steps}`;
      } else {
        detail = 'Запуск упаковки…';
      }
      line = { kind: 'progress', percent: job.progressPercent };
      actions = (
        <>
          {copyButton}
          {cancel}
        </>
      );
    } else if (status === 'SUCCEEDED' && result) {
      // Текст только дополняется: «Готово: N» → «проверяю нарезкой» → «проверено нарезкой · печать ≈…».
      const sliced = checkDone || result.finalSliced !== false;
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
            <>проверяю нарезкой · {stageClock(check.stage, job.receivedAtMs)}</>
          ) : (
            <>проверка нарезкой в очереди · {queueText(check.queuePosition)}</>
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
      detail = joinParts(parts);
      line =
        checkRunning && check
          ? { kind: 'progress', percent: check.progressPercent }
          : checkActive
            ? { kind: 'waiting' }
            : { kind: 'ok' };
      const download = packed3mf ? () => startDownload(packed3mf) : undefined;
      actions = (
        <>
          {copyButton}
          {checkActive ? (
            <button
              type="button"
              className="btn btn-lg btn-cancel"
              disabled={cancelCheckMutation.isPending || job.canManage === false}
              onClick={() => cancelCheckMutation.mutate(job.jobId)}
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
              disabled={checkMutation.isPending || job.canManage === false}
              onClick={() => checkMutation.mutate(job.jobId)}
              data-tip="Нарежет весь стол как слайсер: поддержки соседей, края стола, башня, точное время печати. Несколько минут"
              data-tip-align="end"
            >
              {checkMutation.isPending ? <span className="spinner" aria-hidden="true" /> : <Icon name="layers" />}
              Проверить нарезкой
            </button>
          ) : null}
          <button
            type="button"
            className="btn btn-primary btn-lg btn-main"
            disabled={!download}
            onClick={() => download?.()}
            data-tip={packed3mf ? `${packed3mf.name} · ${formatBytes(packed3mf.sizeBytes)}` : undefined}
            data-tip-align="end"
          >
            <Icon name="download" />
            Скачать 3MF
          </button>
        </>
      );
    } else {
      // FAILED / CANCELLED / EXPIRED
      const failed = status === 'FAILED';
      const headline = job.error ? job.error.split(DETAILS_SEPARATOR)[0] : failed ? 'задача завершилась с ошибкой' : '';
      detail = failed ? (
        <>
          <span className="err">Не удалось: {headline}</span> ·{' '}
          <button type="button" className="linkish" aria-expanded={infoOpen} onClick={() => setInfoOpen(true)}>
            подробности
          </button>
        </>
      ) : status === 'CANCELLED' ? (
        'Задача отменена'
      ) : (
        'Срок хранения истёк'
      );
      line = { kind: failed ? 'fail' : 'off' };
      actions = (
        <>
          {copyButton}
          <button
            type="button"
            className="btn btn-primary btn-lg btn-main"
            onClick={repack}
            disabled={external && !jobFile}
            data-tip="Те же файл и параметры — можно поправить перед запуском"
            data-tip-align="end"
          >
            <Icon name="refresh" />
            Упаковать заново
          </button>
        </>
      );
    }
  } else {
    // Черновик: файл, его загрузка и определение принтера, готовность.
    title = file ? <span className="fname">{file.name}</span> : 'Модель не выбрана';
    if (!file) detail = 'Загрузите слева проект .3mf или модель .stl';
    else if (uploadText) detail = uploadText;
    else if (detection === 'detecting') detail = 'Определяю принтер…';
    else if (parsing) detail = 'Читаю модель…';
    else if (selectedProfile)
      detail = `${selectedProfile.name} · рабочая область ${selectedProfile.bedWidthMm} × ${selectedProfile.bedDepthMm} мм`;
    else detail = detectionError ?? 'Выберите принтер слева';
    const loading = file !== null && (detection === 'detecting' || parsing);
    actions = loading ? (
      <button type="button" className="btn btn-primary btn-lg btn-main" disabled>
        <span className="spinner" aria-hidden="true" />
        Загрузка…
      </button>
    ) : (
      submitButton('Упаковать', startMutation.isPending)
    );
  }
  // «Инфо» слева: у задачи — всегда на месте (и после завершения), у черновика — пустое место той же ширины.
  const lead = viewing ? (
    <button
      type="button"
      className="info-btn"
      aria-expanded={infoOpen}
      onClick={() => setInfoOpen((open) => !open)}
      data-tip="Этапы, время и файлы задачи"
    >
      <Icon name="info" />
      Инфо
    </button>
  ) : (
    <span className="info-slot" aria-hidden="true" />
  );

  /** Текст поверх сцены, пока её нечем заполнить. */
  /** Файл задачи не скачался (истёк срок хранения, нет сети): сцена без модели — сказать прямо. */
  const sceneFailed = external && jobInput.isError ? 'Файл задачи недоступен — стол без модели' : null;
  const sceneBusy = sceneFailed
    ? null
    : external && jobInput.isPending && job?.kind === 'pack'
      ? 'Загружаю файл задачи…'
      : parsing
        ? 'Читаю модель…'
        : !viewing && file && detection === 'detecting' && !viewProfile
          ? 'Определяю принтер…'
          : null;
  const errorText = job?.status === 'FAILED' && job.error ? job.error : null;

  return (
    <div className={`work${calibOpen ? ' with-side' : ''}`}>
      <form id="pack-form" className="form-col" onSubmit={handleSubmit}>
        <FileDrop
          file={shownFile}
          onFileChange={handleFileChange}
          disabled={jobActive || startMutation.isPending}
          allowStl
        />
        {!viewing && detectMutation.upload ? (
          <div className="file-progress">
            <div className="track">
              <i
                style={{
                  ['--p' as string]: `${(detectMutation.upload.sent / Math.max(1, detectMutation.upload.total)) * 100}%`,
                }}
              />
            </div>
            <span>
              Загрузка <Num>{formatBytes(detectMutation.upload.sent)}</Num> из{' '}
              <Num>{formatBytes(detectMutation.upload.total)}</Num>
            </span>
          </div>
        ) : !viewing && file && (detection === 'detecting' || parsing) ? (
          <div className="file-progress">
            <div className="track waiting">
              <i />
            </div>
            <span>{detection === 'detecting' ? 'Определяю принтер…' : 'Читаю модель…'}</span>
          </div>
        ) : null}
        {!viewing && detectionError ? <div className="error-block">{detectionError}</div> : null}
        <fieldset className="form-lock" disabled={formLocked}>
          {/* 3MF известного принтера — выбор заблокирован (сервер примет только его), неизвестного — только
            свой стол; STL и «до файла» — любой принтер каталога. Не Field: label-обёртка уводила бы клик по опции в поле. */}
          <div className="field">
            <label className="field-label" htmlFor="pack-printer">
              Принтер
            </label>
            {external ? (
              <output id="pack-printer" className="input">
                {job?.profile?.name ?? 'Принтер задачи'}
              </output>
            ) : (
              <PrinterPicker
                id="pack-printer"
                profiles={profiles}
                value={choice?.kind === 'custom' ? CUSTOM_PRINTER : (choice?.id ?? null)}
                onChange={setPicked}
                locked={choice?.kind === 'detected'}
                customOnly={unknown3mf}
                disabled={busy || (file !== null && !stl && detection === 'detecting')}
                placeholder={profilesQuery.isPending ? 'Загрузка…' : 'Выберите принтер'}
              />
            )}
            {pickerHint && !viewing ? <span className="field-hint">{pickerHint}</span> : null}
          </div>
          {customBedFields}
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
                checked={sliceNow || windowOn}
                onChange={(event) => setSliceNow(event.target.checked)}
                disabled={busy || windowOn}
              />
              <span className="check-text">
                Сразу проверить нарезкой
                <small className="field-hint">
                  {windowOn
                    ? 'окно печати меряется нарезками — стол режется всегда'
                    : 'дольше на несколько минут; можно и потом — кнопкой «Проверить нарезкой»'}
                </small>
              </span>
            </label>
          </div>
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
        </fieldset>
        {viewing && job && !ACTIVE.includes(job.status) ? (
          // Параметры задачи слева только для чтения; поменять их — здесь же, новой упаковкой.
          <div className="repack">
            <button type="button" className="btn btn-block" onClick={repack} disabled={external && !jobFile}>
              <Icon name="refresh" />
              Упаковать заново
            </button>
            <span className="field-hint">Тот же файл, параметры снова можно менять. Получится новая задача.</span>
          </div>
        ) : null}
      </form>

      <section className="main-col">
        <StatusStrip title={title} detail={detail} line={line} lineLabel={lineLabel} actions={actions} lead={lead} />
        <div className="scene-wrap">
          {viewProfile ? (
            <ErrorBoundary
              fallback={
                <div className="scene scene-fallback">
                  Не удалось показать стол: браузер без WebGL или устаревшая версия страницы — обновите страницу
                </div>
              }
            >
              <Suspense fallback={<div className="scene" />}>
                <BedScene
                  profile={viewProfile}
                  model={model}
                  mode={sceneMode}
                  clip={
                    running && job?.stage
                      ? sceneClip(job.stage.id, job.progressPercent)
                      : checkRunning && check?.stage
                        ? sceneClip(check.stage.id, check.progressPercent)
                        : 1
                  }
                  searching={searching}
                  caption={
                    // Слишком большой файл — без предпросмотра (без тоста: упаковать его всё равно можно).
                    shownFile && shownFile.size > PREVIEW_MAX_BYTES
                      ? 'Модель слишком большая для предпросмотра'
                      : searching
                        ? 'поиск раскладки'
                        : model
                          ? 'колесо — масштаб, перетаскивание — поворот, правая кнопка — сдвиг'
                          : ''
                  }
                  placements={placed.placements}
                  footprintLoops={placed.footprintLoops}
                  tower={placed.tower}
                  usableOverride={placed.bed}
                  readout={model ? `Модель: ${model.size.map((v) => Math.round(v)).join(' × ')} мм` : ''}
                  objectCount={resultCount}
                />
              </Suspense>
            </ErrorBoundary>
          ) : (
            <div className="scene scene-empty">
              {sceneBusy ? null : 'Здесь появится стол выбранного принтера и модель'}
            </div>
          )}
          {sceneBusy || sceneFailed ? (
            <div className="scene-busy" aria-live="polite">
              <span>
                {sceneBusy ? <span className="spinner" aria-hidden="true" /> : <Icon name="alert" />}
                {sceneBusy ?? sceneFailed}
              </span>
            </div>
          ) : null}
          {infoOpen && viewing && job ? (
            <ScenePopover title={`Задача #${jobId!.slice(0, 8)}`} onClose={closeInfo}>
              {job.status === 'RETRY_WAIT' || job.status === 'LOST' ? (
                <p className="hint info-note">
                  {job.phaseMessage ?? 'Задача запущена заново'}: этапы начнутся сначала.
                </p>
              ) : null}
              {job.stage ? (
                <StageList
                  stage={job.stage}
                  running={running}
                  failed={job.status === 'FAILED'}
                  elapsedNow={stageElapsed(job.stage, job.receivedAtMs, nowAfter(job.receivedAtMs))}
                  finishedAt={job.finishedAt}
                />
              ) : (
                <p className="hint">
                  {ACTIVE.includes(job.status) ? 'Этапы появятся, когда задача начнётся.' : 'Этапов нет.'}
                </p>
              )}
              {errorText ? <JobErrorDetails error={errorText} /> : null}
              {check ? (
                <>
                  <div className="group-title">Проверка нарезкой</div>
                  {check.stage ? (
                    <StageList
                      stage={check.stage}
                      running={checkRunning}
                      failed={check.status === 'FAILED'}
                      elapsedNow={stageElapsed(check.stage, job.receivedAtMs, nowAfter(job.receivedAtMs))}
                      finishedAt={check.finishedAt}
                    />
                  ) : (
                    <p className="hint">
                      {checkActive ? 'В очереди.' : check.status === 'CANCELLED' ? 'Остановлена.' : 'Этапов нет.'}
                    </p>
                  )}
                  {check.status === 'FAILED' && check.error ? <JobErrorDetails error={check.error} /> : null}
                </>
              ) : null}
              {!ACTIVE.includes(job.status) ? (
                <>
                  <div className="group-title">Файлы</div>
                  <JobFiles
                    artifacts={
                      checkDone && check
                        ? [...job.artifacts.filter((a) => a.name === 'log.txt'), ...check.artifacts]
                        : job.artifacts
                    }
                    labelOf={(artifact) =>
                      checkDone && artifact.name === 'log.txt'
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
          ) : null}
        </div>
      </section>

      {/* Скрыта, а не размонтирована: jobId калибровки живёт в её состоянии и терялся при закрытии. */}
      <aside className="side-col" hidden={!calibOpen}>
        <CalibrationPage embedded onClose={() => setCalibOpen(false)} />
      </aside>
    </div>
  );
}

/** Ошибка задачи или проверки: заголовок и сырой хвост журнала (бэкенд склеивает их через DETAILS_SEPARATOR). */
function JobErrorDetails({ error }: { error: string }) {
  const at = error.indexOf(DETAILS_SEPARATOR);
  const headline = at >= 0 ? error.slice(0, at) : error;
  const details = at >= 0 ? error.slice(at + DETAILS_SEPARATOR.length) : null;
  return (
    <>
      <div className="error-block">{headline}</div>
      {details ? <pre className="error-details">{details}</pre> : null}
    </>
  );
}

/** Этапы с длительностями: пройденные, идущий (со спиннером) и ещё предстоящие. */
function StageList({
  stage,
  running,
  failed,
  elapsedNow,
  finishedAt,
}: {
  stage: JobStage;
  running: boolean;
  failed: boolean;
  elapsedNow: number;
  finishedAt: string | null;
}) {
  const rows = stageRows(stage, {
    running,
    failed,
    elapsedNow,
    finishedAtMs: finishedAt ? Date.parse(finishedAt) : null,
  });
  return (
    <ol className="stage-list">
      {rows.map((row) => (
        <li key={row.key} className={row.state}>
          {row.state === 'current' ? <span className="spin" aria-hidden="true" /> : <i aria-hidden="true" />}
          {row.label}
          <span className="t">
            {row.seconds == null ? '' : formatDuration(row.seconds)}
          </span>
        </li>
      ))}
    </ol>
  );
}
