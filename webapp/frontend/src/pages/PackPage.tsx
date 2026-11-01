import { lazy, Suspense, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { FormEvent, ReactNode } from 'react';
import { useMutation, useQuery } from '@tanstack/react-query';
import { ApiError } from '@/api/http';
import { deleteJob, downloadJobInput, fetchProfiles, finishPackJob, jobLink, startPackJob } from '@/api/services';
import type { JobStatusResponse } from '@/api/types';
import AmsCorrectionFields from '@/components/AmsCorrectionFields';
import ErrorBoundary from '@/components/ErrorBoundary';
import FileDrop from '@/components/FileDrop';
import Field from '@/components/Field';
import Icon from '@/components/Icon';
import JobFiles, { startDownload, visibleArtifacts } from '@/components/JobFiles';
import ScenePopover from '@/components/ScenePopover';
import SplitButton from '@/components/SplitButton';
import StatusStrip, { type StripLine } from '@/components/StatusStrip';
import PrinterPicker, { CUSTOM_PRINTER } from '@/components/PrinterPicker';
import CalibrationPage from '@/pages/CalibrationPage';
import { sceneLayout } from '@/scene/layout';
import type { ModelMesh } from '@/scene/model3d';
import { parseModelInWorker, PREVIEW_MAX_BYTES } from '@/scene/parseModelAsync';
import { buildNozzleClean, buildSupportOverlap, numberInRange, packTimeFields, parseNozzleClean } from '@/utils/forms';
import { formatBytes, formatHoursMinutes } from '@/utils/format';
import { objectsText, sceneClip, stageElapsed, stageItems, stageTimeLine, stripStage } from '@/utils/progress';
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
  /** Панель поверх сцены. */
  const [pop, setPop] = useState<'stages' | 'files' | 'error' | null>(null);
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

  const finishMutation = useMutation({
    mutationFn: finishPackJob,
    onSuccess: (response) => {
      // Доводка режет тот же вход: его модель уже на сцене, если родитель запущен отсюда.
      if (jobId && startedHere.has(jobId)) setStartedHere((ids) => new Set(ids).add(response.jobId));
      navigate(`/pack/${response.jobId}`);
    },
    onError: (error) => {
      pushToast(error instanceof ApiError ? error.message : 'Не удалось запустить нарезку', 'error');
    },
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

  // ───── Строка статуса: что происходит и одна главная кнопка ─────
  const now = useNow(running);
  const copyLink = useCallback(() => {
    if (!jobId) return;
    navigator.clipboard
      .writeText(jobLink(jobId))
      .then(() => pushToast('Ссылка скопирована', 'success'))
      .catch(() => pushToast('Не удалось скопировать ссылку', 'error'));
  }, [jobId]);
  const closePop = useCallback(() => setPop(null), []);
  const togglePop = (kind: 'stages' | 'files' | 'error') => setPop((open) => (open === kind ? null : kind));
  const popLink = (kind: 'stages' | 'files' | 'error', label: string) => (
    <button type="button" className="linkish" aria-expanded={pop === kind} onClick={() => togglePop(kind)}>
      {label}
    </button>
  );
  const newPack = () => {
    setFile(null);
    setBedInput(EMPTY_BED);
    navigate('/pack');
  };
  /** «Упаковать заново»: черновик с теми же файлом и параметрами; у задачи по ссылке файл — её вход. */
  const repack = () => {
    if (external && jobFile) setFile(jobFile);
    navigate('/pack');
  };
  const copyButton = jobId ? (
    <button
      type="button"
      className="icon-btn bordered"
      aria-label="Скопировать ссылку на задачу"
      data-tip="Скопировать ссылку на задачу"
      data-tip-align="end"
      onClick={copyLink}
    >
      <Icon name="copy" />
    </button>
  ) : null;
  const packed3mf = job ? visibleArtifacts(job.artifacts).find((a) => a.name.endsWith('.3mf')) : undefined;
  const moreItems = [
    { label: 'Файлы задачи', icon: 'file' as const, onSelect: () => setPop('files') },
    'separator' as const,
    { label: 'Упаковать заново', icon: 'refresh' as const, onSelect: repack, disabled: external && !jobFile },
    { label: 'Новая упаковка', icon: 'plus' as const, onSelect: newPack },
  ];
  const uploadText = detectMutation.upload ? (
    <>
      Загрузка файла ·{' '}
      <Num>{Math.round((detectMutation.upload.sent / Math.max(1, detectMutation.upload.total)) * 100)}</Num> %
    </>
  ) : null;

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

  if (viewing && jobGone) {
    title = 'Задача недоступна';
    detail = 'Удалена или истёк срок хранения файлов';
    actions = (
      <button
        type="button"
        className="btn btn-primary btn-lg btn-main"
        onClick={newPack}
        data-tip="Начать с чистой формы"
        data-tip-align="end"
      >
        <Icon name="plus" />
        Новая упаковка
      </button>
    );
  } else if (viewing) {
    const status = job?.status;
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
      title = jobQuery.isError ? 'Нет связи с сервером' : 'Получаю статус задачи…';
      detail = jobQuery.isError && jobQuery.error instanceof Error ? jobQuery.error.message : '';
      line = { kind: 'waiting' };
      actions = copyButton;
    } else if (status === 'QUEUED' || status === 'RETRY_WAIT' || status === 'LOST') {
      title = status === 'QUEUED' ? 'В очереди' : 'Повтор после сбоя исполнителя';
      detail =
        job.queuePosition != null ? (
          <>
            место <Num>{job.queuePosition}</Num>
            {job.estimatedWaitFromSeconds != null && job.estimatedWaitToSeconds != null ? (
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
      line = { kind: 'waiting' };
      actions = (
        <>
          {cancel}
          {copyButton}
        </>
      );
    } else if (status === 'RUNNING' || status === 'FINALIZING') {
      const stage = job.stage;
      if (stage) {
        const strip = stripStage(stage);
        const elapsed = stageElapsed(stage, job.receivedAtMs, Math.max(now, job.receivedAtMs ?? 0));
        title = strip.label;
        detail = (
          <>
            этап <Num>{strip.step}</Num> из <Num>{strip.steps}</Num> · {stageTimeLine(elapsed, stage.expectedSeconds)} ·{' '}
            {popLink('stages', 'этапы')}
          </>
        );
        lineLabel = `${strip.label}, этап ${strip.step} из ${strip.steps}`;
      } else {
        title = 'Запуск упаковки…';
        detail = 'исполнитель готовит задачу';
      }
      line = { kind: 'progress', percent: job.progressPercent };
      actions = (
        <>
          {cancel}
          {copyButton}
        </>
      );
    } else if (status === 'SUCCEEDED' && job.pack) {
      const info = job.pack;
      title = `${objectsText(info.objectCount)} на столе`;
      line = { kind: 'ok' };
      const parts: ReactNode[] = [];
      // Коротко: строка не переносится, а ссылка «файлы» в конце не должна уйти под многоточие.
      if (info.finalSliced === false) parts.push('стол не нарезан целиком');
      if (info.time) {
        parts.push(<>печать ≈{formatHoursMinutes(info.time.correctedSeconds)}</>);
        if (info.time.windowHours != null)
          parts.push(
            <>
              окно <Num>{info.time.windowHours}</Num> ч: <Num>{info.time.chosenCount}</Num> из{' '}
              <Num>{info.time.fullCount}</Num>
            </>
          );
      }
      if (info.supportsDetected) parts.push('с поддержками, без поворотов');
      parts.push(popLink('files', 'файлы'));
      detail = parts.map((part, index) => (
        <span key={index}>
          {index ? ' · ' : ''}
          {part}
        </span>
      ));
      const download = packed3mf ? () => startDownload(packed3mf) : undefined;
      actions =
        info.finalSliced === false ? (
          <>
            <SplitButton
              main={
                <button
                  type="button"
                  className="btn btn-primary btn-lg btn-main"
                  disabled={finishMutation.isPending || job.canManage === false}
                  onClick={() => finishMutation.mutate(job.jobId)}
                  data-tip="Проверить поддержки на всём столе и посчитать время печати"
                  data-tip-align="end"
                >
                  {finishMutation.isPending ? <span className="spinner" aria-hidden="true" /> : <Icon name="layers" />}
                  Нарезать стол
                </button>
              }
              items={[
                {
                  label: 'Скачать 3MF',
                  icon: 'download',
                  note: packed3mf ? formatBytes(packed3mf.sizeBytes) : undefined,
                  onSelect: () => download?.(),
                  disabled: !download,
                },
                ...moreItems,
              ]}
            />
            {copyButton}
          </>
        ) : (
          <>
            <SplitButton
              main={
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
              }
              items={moreItems}
            />
            {copyButton}
          </>
        );
    } else {
      // FAILED / CANCELLED / EXPIRED
      const failed = status === 'FAILED';
      title = failed ? 'Упаковка не удалась' : status === 'CANCELLED' ? 'Задача отменена' : 'Срок хранения истёк';
      const headline = job.error ? job.error.split(DETAILS_SEPARATOR)[0] : failed ? 'Задача завершилась с ошибкой' : '';
      detail = (
        <>
          {headline ? <span className="err">{headline}</span> : null}
          {failed ? <> · {popLink('error', 'подробности')}</> : null}
        </>
      );
      line = { kind: failed ? 'fail' : 'off' };
      actions = (
        <>
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
          {copyButton}
        </>
      );
    }
  } else {
    // Черновик: файл, его загрузка и определение принтера, готовность.
    title = file ? file.name : 'Модель не выбрана';
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

  /** Текст поверх сцены, пока её нечем заполнить. */
  const sceneBusy =
    external && jobInput.isPending && job?.kind === 'pack'
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
                Сразу нарезать весь стол
                <small className="field-hint">
                  {windowOn
                    ? 'окно печати меряется нарезками — стол режется всегда'
                    : 'дольше на несколько минут; без галочки — кнопкой после упаковки'}
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
        {viewing ? <p className="field-hint">Параметры этой задачи. Чтобы изменить — «Упаковать заново».</p> : null}
      </form>

      <section className="main-col">
        <StatusStrip title={title} detail={detail} line={line} lineLabel={lineLabel} actions={actions} />
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
                  clip={running && job?.stage ? sceneClip(job.stage.id, job.progressPercent) : 1}
                  caption={
                    // Слишком большой файл — без предпросмотра (без тоста: упаковать его всё равно можно).
                    shownFile && shownFile.size > PREVIEW_MAX_BYTES
                      ? 'Модель слишком большая для предпросмотра'
                      : model
                        ? 'колесо — масштаб, перетаскивание — поворот, правая кнопка — сдвиг'
                        : ''
                  }
                  placements={placed.placements}
                  footprintLoops={placed.footprintLoops}
                  tower={placed.tower}
                  usableOverride={placed.bed}
                  readout={
                    placed.placements.length
                      ? `${objectsText(placed.placements.length)} на столе`
                      : model
                        ? `${model.size.map((v) => Math.round(v)).join(' × ')} мм`
                        : ''
                  }
                />
              </Suspense>
            </ErrorBoundary>
          ) : (
            <div className="scene scene-empty">
              {sceneBusy ? null : 'Здесь появится стол выбранного принтера и модель'}
            </div>
          )}
          {sceneBusy ? (
            <div className="scene-busy" aria-live="polite">
              <span>
                <span className="spinner" aria-hidden="true" />
                {sceneBusy}
              </span>
            </div>
          ) : null}
          {pop === 'stages' && running && job?.stage ? (
            <ScenePopover title="Этапы" onClose={closePop}>
              <ol className="stage-list">
                {stageItems(job.stage).map((item) => (
                  <li key={item.key} className={item.state}>
                    <i aria-hidden="true" />
                    {item.label}
                    {item.state === 'current' ? (
                      <span className="t">
                        {stageTimeLine(
                          stageElapsed(job.stage!, job.receivedAtMs, Math.max(now, job.receivedAtMs ?? 0)),
                          null
                        )}
                      </span>
                    ) : null}
                  </li>
                ))}
              </ol>
            </ScenePopover>
          ) : null}
          {pop === 'files' && job && !ACTIVE.includes(job.status) ? (
            <ScenePopover title="Файлы задачи" onClose={closePop}>
              <JobFiles artifacts={job.artifacts} expiresAt={job.expiresAt} />
            </ScenePopover>
          ) : null}
          {pop === 'error' && errorText ? (
            <ScenePopover title="Что пошло не так" onClose={closePop}>
              <JobErrorDetails error={errorText} job={job} />
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

/** Ошибка задачи: заголовок и сырой хвост журнала (бэкенд склеивает их через DETAILS_SEPARATOR) + файлы. */
function JobErrorDetails({ error, job }: { error: string; job: JobStatusResponse | null }) {
  const at = error.indexOf(DETAILS_SEPARATOR);
  const headline = at >= 0 ? error.slice(0, at) : error;
  const details = at >= 0 ? error.slice(at + DETAILS_SEPARATOR.length) : null;
  return (
    <>
      <div className="error-block">{headline}</div>
      {details ? <pre className="error-details">{details}</pre> : null}
      {job ? <JobFiles artifacts={job.artifacts} expiresAt={job.expiresAt} /> : null}
    </>
  );
}
