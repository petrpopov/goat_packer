import { lazy, Suspense, useEffect, useMemo, useState } from 'react';
import type { FormEvent } from 'react';
import { useMutation, useQuery } from '@tanstack/react-query';
import { ApiError } from '@/api/http';
import { fetchProfiles, finishPackJob, startPackJob } from '@/api/services';
import AmsCorrectionFields from '@/components/AmsCorrectionFields';
import ErrorBoundary from '@/components/ErrorBoundary';
import FileDrop from '@/components/FileDrop';
import Field from '@/components/Field';
import Icon from '@/components/Icon';
import JobCard from '@/components/JobCard';
import PrinterPicker, { CUSTOM_PRINTER } from '@/components/PrinterPicker';
import CalibrationPage from '@/pages/CalibrationPage';
import { sceneLayout } from '@/scene/layout';
import type { ModelMesh } from '@/scene/model3d';
import { parseModelInWorker, PREVIEW_MAX_BYTES } from '@/scene/parseModelAsync';
import { buildNozzleClean, buildSupportOverlap, numberInRange, packTimeFields } from '@/utils/forms';
import { objectsText, sceneClip } from '@/utils/progress';
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
  /** Сразу нарезать весь стол; иначе — по кнопке «Нарезать стол» после упаковки (минуты ожидания). */
  const [sliceNow, setSliceNow] = useState(false);
  const [windowHours, setWindowHours] = useState('12');
  const [marginPct, setMarginPct] = useState('10');
  const [supportOverlap, setSupportOverlap] = useState('');
  const [jobId, setJobId] = useState<string | null>(null);
  const [model, setModel] = useState<ModelMesh | null>(null);
  /** Выбор в PrinterPicker (id или CUSTOM_PRINTER); переживает смену файла, определение по 3MF его перекрывает. */
  const [picked, setPicked] = useState<string | null>(null);
  const [bedInput, setBedInput] = useState<CustomBedInput>(EMPTY_BED);

  // Слишком большой файл — без предпросмотра (без тоста: упаковать его всё равно можно).
  const modelTooLarge = file !== null && file.size > PREVIEW_MAX_BYTES;

  // Предпросмотр модели в сцене разбирается в браузере из того же файла, что уйдёт на сервер —
  // в воркере, чтобы разбор крупного 3MF не замораживал страницу.
  useEffect(() => {
    let cancelled = false;
    let cancelParse: (() => void) | null = null;
    setModel(null);
    if (!file || file.size > PREVIEW_MAX_BYTES) return;
    file
      .arrayBuffer()
      .then((buffer) => {
        if (cancelled) return null;
        const parse = parseModelInWorker(file.name, buffer);
        cancelParse = parse.cancel;
        return parse.promise;
      })
      .then((mesh) => {
        if (!cancelled && mesh) setModel(mesh);
      })
      .catch((error: unknown) => {
        if (!cancelled) pushToast(error instanceof Error ? error.message : 'Не удалось прочитать модель', 'error');
      });
    return () => {
      cancelled = true;
      cancelParse?.();
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
  const jobGone = jobQuery.error instanceof ApiError && jobQuery.error.status === 404;
  const jobActive =
    Boolean(jobId) &&
    !jobGone &&
    (!job || ['QUEUED', 'RUNNING', 'FINALIZING', 'LOST', 'RETRY_WAIT'].includes(job.status));
  // Новый файл без идущей задачи: раскладка прошлой задачи к нему не относится — сцена не должна
  // ставить новую модель на её места.
  // Размеры своего стола от прошлого файла к новому не относятся: поля пустеют, стол проекта нового
  // 3MF (если придёт) заполнит их снова.
  const handleFileChange = (next: File | null) => {
    setFile(next);
    setBedInput(EMPTY_BED);
    if (!jobActive) setJobId(null);
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
    // Свой стол: selectedProfile есть, только когда все размеры в границах (parseCustomBed).
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
      setJobId(response.jobId);
      pushToast('Нарезка стола запущена', 'success');
    },
    onError: (error) => {
      pushToast(error instanceof ApiError ? error.message : 'Не удалось запустить нарезку', 'error');
    },
  });

  const successContent =
    job?.status === 'SUCCEEDED' && job.pack ? (
      <div className="form-actions">
        <p className="result-title">Упаковано объектов: {job.pack.objectCount}</p>
        {job.pack.finalSliced === false ? (
          <>
            <p className="field-hint">
              Раскладка проверена по пробной нарезке нескольких объектов. Полная нарезка стола займёт несколько минут:
              она проверит поддержки на всём столе и посчитает время печати.
            </p>
            <button
              type="button"
              className="btn btn-primary"
              disabled={finishMutation.isPending}
              onClick={() => finishMutation.mutate(job.jobId)}
            >
              {finishMutation.isPending ? <span className="spinner" aria-hidden="true" /> : null}
              Нарезать стол
            </button>
          </>
        ) : null}
        {/* Расписание своего стола бэкенд не принимает: у разового профиля нет файла в каталоге. */}
        {job.pack.profileId === 'custom' ? null : (
          <button
            type="button"
            className="btn btn-primary"
            onClick={() => sendToSchedule({ packJobId: job.jobId, profileId: job.pack?.profileId ?? profileId })}
          >
            В расписание <Icon name="arrow-right" />
          </button>
        )}
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
    projectBed !== null && (key === 'width' || key === 'depth' || (key === 'height' && projectBed.printableHeight != null));
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
        hint={outOfRange ? `${min}–${max} мм` : marginTooLarge ? 'отступ больше половины стола' : fixed ? 'из проекта' : hint}
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

  return (
    <div className={`work${calibOpen ? ' with-side' : ''}`}>
      <form id="pack-form" className="form-col" onSubmit={handleSubmit}>
        <FileDrop file={file} onFileChange={handleFileChange} disabled={busy} allowStl />
        {detectionError ? <div className="error-block">{detectionError}</div> : null}
        {/* 3MF известного принтера — выбор заблокирован (сервер примет только его), неизвестного — только
            свой стол; STL и «до файла» — любой принтер каталога. Не Field: label-обёртка уводила бы клик по опции в поле. */}
        <div className="field">
          <label className="field-label" htmlFor="pack-printer">
            Принтер
          </label>
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
          {pickerHint ? <span className="field-hint">{pickerHint}</span> : null}
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
      </form>

      <section className="main-col">
        <div className="action-bar">
          <div className="title">
            <h1>{file ? file.name : 'Модель не выбрана'}</h1>
            <p>
              {selectedProfile
                ? `${selectedProfile.name} · рабочая область ${selectedProfile.bedWidthMm} × ${selectedProfile.bedDepthMm} мм`
                : 'Загрузите слева проект .3mf или модель .stl и выберите принтер'}
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
          <ErrorBoundary
            fallback={
              <div className="scene scene-fallback">
                Не удалось показать стол: браузер без WebGL или устаревшая версия страницы — обновите страницу
              </div>
            }
          >
            <Suspense fallback={<div className="scene" />}>
              <BedScene
                profile={selectedProfile}
                model={model}
                mode={sceneMode}
                clip={running && job?.stage ? sceneClip(job.stage.id, job.progressPercent) : 1}
                caption={
                  modelTooLarge
                    ? 'Модель слишком большая для предпросмотра'
                    : sceneMode === 'done'
                      ? 'перетащите, чтобы повернуть'
                      : model
                        ? 'модель и её проекция на стол'
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
        )}
      </section>

      {/* Скрыта, а не размонтирована: jobId калибровки живёт в её состоянии и терялся при закрытии. */}
      <aside className="side-col" hidden={!calibOpen}>
        <CalibrationPage embedded onClose={() => setCalibOpen(false)} />
      </aside>
    </div>
  );
}
