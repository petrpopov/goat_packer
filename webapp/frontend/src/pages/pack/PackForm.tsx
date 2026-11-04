import type { FormEvent, ReactNode } from 'react';
import type { PrinterProfile, SlicerOption } from '@/api/types';
import AmsCorrectionFields from '@/components/AmsCorrectionFields';
import Field from '@/components/Field';
import FileDrop from '@/components/FileDrop';
import NumberInput from '@/components/NumberInput';
import PrinterPicker, { CUSTOM_PRINTER } from '@/components/PrinterPicker';
import type { useAmsCorrection } from '@/hooks/useAmsCorrection';
import { formatBytes } from '@/utils/format';
import { inRange, LIMITS, rangeHint, type NozzleCleanMode } from '@/utils/forms';
import { slicerLabel } from '@/utils/printers';
import CustomBedFields from './CustomBedFields';
import { DEFAULT_TOWER_WIDTH_MM, towerWidthValid, type PackFormValues, type PackRequestParts } from './packFormValues';
import { PACK_FORM_ID } from './PackStatusStrip';
import type { PrinterSelection } from './usePrinterSelection';

type Update = <K extends keyof PackFormValues>(key: K, value: PackFormValues[K]) => void;

interface PackFormProps {
  values: PackFormValues;
  update: Update;
  /** Файл в поле: черновика или открытой задачи. */
  shownFile: File | null;
  onFileChange: (file: File | null) => void;
  /** Идёт задача или отправка: поля и файл недоступны. */
  busy: boolean;
  /** Форма только для чтения: открыта задача, идёт отправка или файл ещё загружается. */
  locked: boolean;
  /** Открыта задача (не черновик). */
  viewing: boolean;
  /** Принтер задачи, открытой по ссылке (вместо выбора); undefined — черновик или своя задача. */
  jobPrinterName?: string;
  /** Файл черновика ещё читается в воркере. */
  parsing: boolean;
  selection: PrinterSelection;
  profiles: PrinterProfile[];
  profilesLoading: boolean;
  onPick: (value: string) => void;
  parts: PackRequestParts;
  ams: ReturnType<typeof useAmsCorrection>;
  onCalibrate: () => void;
  onSubmit: (event: FormEvent<HTMLFormElement>) => void;
  /** Слайсер STL: варианты принтера и выбор; null — не STL (3MF режется своим слайсером). */
  slicer: { options: SlicerOption[]; value: string } | null;
  /** «Упаковать заново» под формой у завершённой задачи; null — нет. */
  repack: ReactNode;
}

/** Левая колонка упаковки: файл, принтер, раскладка, чистка, нарезка, окно печати, дополнительное. */
export default function PackForm(props: PackFormProps) {
  const { values, update, busy, viewing, selection, parts } = props;
  const nozzleCleanAvailable = selection.selectedProfile === null || selection.selectedProfile.supportsNozzleClean;
  return (
    <form id={PACK_FORM_ID} className="form-col" onSubmit={props.onSubmit}>
      <FileDrop file={props.shownFile} onFileChange={props.onFileChange} disabled={busy} allowStl />
      {viewing ? null : (
        <FileProgress selection={selection} parsing={props.parsing} hasFile={props.shownFile !== null} />
      )}
      {!viewing && selection.detectionError ? <div className="error-block">{selection.detectionError}</div> : null}
      <fieldset className="form-lock" disabled={props.locked}>
        <PrinterField {...props} />
        {props.slicer ? (
          <SlicerField
            options={props.slicer.options}
            value={props.slicer.value}
            onChange={(kind) => update('slicer', kind)}
            disabled={busy}
          />
        ) : null}
        {selection.custom ? (
          <CustomBedFields
            bed={values.bed}
            onChange={(bed) => update('bed', bed)}
            parsed={selection.customBed}
            projectBed={selection.projectBed}
            disabled={busy}
          />
        ) : null}
        <Field
          label="Раскладка"
          hint={values.layout === 'grid' ? 'быстро и равномерно, без поворотов' : 'подбор с поворотами и нависаниями'}
        >
          <div className="seg" role="group" aria-label="Раскладка">
            <button
              type="button"
              aria-pressed={values.layout === 'search'}
              onClick={() => update('layout', 'search')}
              disabled={busy}
            >
              Плотная
            </button>
            <button
              type="button"
              aria-pressed={values.layout === 'grid'}
              onClick={() => update('layout', 'grid')}
              disabled={busy}
            >
              Сетка
            </button>
          </div>
        </Field>
        <NozzleCleanFields
          values={values}
          update={update}
          available={nozzleCleanAvailable}
          invalid={parts.nozzleCleanInvalid}
          disabled={busy}
        />
        <div className="form-group">
          <label className="check">
            <input
              type="checkbox"
              checked={values.sliceNow || values.windowOn}
              onChange={(event) => update('sliceNow', event.target.checked)}
              disabled={busy || values.windowOn}
            />
            <span className="check-text">
              Сразу проверить нарезкой
              <small className="field-hint">
                {values.windowOn
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
              checked={values.windowOn}
              onChange={(event) => update('windowOn', event.target.checked)}
              disabled={busy}
            />
            <span className="check-text">
              Уложиться в окно печати
              <small className="field-hint">без окна стол заполняется максимально</small>
            </span>
          </label>
          {values.windowOn && (
            <div className="nested">
              <PrintWindowFields values={values} update={update} disabled={busy} />
              <AmsCorrectionFields
                available={props.ams.available}
                on={props.ams.on}
                onToggle={props.ams.setOn}
                seconds={props.ams.seconds}
                onSecondsChange={props.ams.setSeconds}
                disabled={busy}
                onCalibrate={props.onCalibrate}
              />
            </div>
          )}
        </div>
        <details className="params form-group">
          <summary className="params-summary">Дополнительно</summary>
          <AdvancedFields
            values={values}
            update={update}
            supportInvalid={parts.supportInvalid}
            custom={selection.custom}
            profile={selection.selectedProfile}
            disabled={busy}
          />
        </details>
      </fieldset>
      {props.repack}
    </form>
  );
}

/** Под полем файла: полоса отправки («1.4 из 2.6 МБ»), затем «Определяю принтер…» / «Читаю модель…». */
function FileProgress({
  selection,
  parsing,
  hasFile,
}: {
  selection: PrinterSelection;
  parsing: boolean;
  hasFile: boolean;
}) {
  const { upload, detection } = selection;
  if (upload) {
    return (
      <div className="file-progress">
        <div className="track">
          <i style={{ ['--p' as string]: upload.sent / Math.max(1, upload.total) }} />
        </div>
        <span>
          Загрузка <span className="num">{formatBytes(upload.sent)}</span> из{' '}
          <span className="num">{formatBytes(upload.total)}</span>
        </span>
      </div>
    );
  }
  if (!hasFile || (detection !== 'detecting' && !parsing)) return null;
  return (
    <div className="file-progress">
      <div className="track waiting">
        <i />
      </div>
      <span>{detection === 'detecting' ? 'Определяю принтер…' : 'Читаю модель…'}</span>
    </div>
  );
}

/**
 * 3MF известного принтера — выбор заблокирован (сервер примет только его), неизвестного — только
 * свой стол; STL и «до файла» — любой принтер каталога. Не Field: label-обёртка уводила бы клик по опции в поле.
 */
function PrinterField({ selection, profiles, profilesLoading, onPick, busy, viewing, jobPrinterName }: PackFormProps) {
  const { choice } = selection;
  return (
    <div className="field">
      <label className="field-label" htmlFor="pack-printer">
        Принтер
      </label>
      {jobPrinterName !== undefined ? (
        <output id="pack-printer" className="input">
          {jobPrinterName}
        </output>
      ) : (
        <PrinterPicker
          id="pack-printer"
          profiles={profiles}
          value={choice?.kind === 'custom' ? CUSTOM_PRINTER : (choice?.id ?? null)}
          onChange={onPick}
          locked={choice?.kind === 'detected'}
          customOnly={selection.unknown3mf}
          disabled={busy || selection.detecting3mf}
          placeholder={profilesLoading ? 'Загрузка…' : 'Выберите принтер'}
        />
      )}
      {selection.pickerHint && !viewing ? <span className="field-hint">{selection.pickerHint}</span> : null}
    </div>
  );
}

/**
 * Слайсер, в котором пользователь откроет и напечатает стол (только STL: 3MF режется слайсером, для
 * которого сохранён). Им же сервер собирает проект и проверяет раскладку. Один вариант — подпись
 * без выбора. Не Field: label-обёртка уводила бы клик по кнопке в первую.
 */
function SlicerField({
  options,
  value,
  onChange,
  disabled,
}: {
  options: SlicerOption[];
  value: string;
  onChange: (kind: string) => void;
  disabled: boolean;
}) {
  const hint = 'в нём откроете стол: проект и проверки — под него';
  if (options.length <= 1) {
    const only = options[0] ?? { kind: value, gui: '' };
    return (
      <div className="field">
        <span className="field-label">Слайсер</span>
        <output className="input">{slicerLabel(only)}</output>
        <span className="field-hint">{hint}</span>
      </div>
    );
  }
  return (
    <div className="field">
      <span className="field-label">Слайсер</span>
      <div className="seg" role="group" aria-label="Слайсер">
        {options.map((option) => (
          <button
            key={option.kind}
            type="button"
            aria-pressed={value === option.kind}
            onClick={() => onChange(option.kind)}
            disabled={disabled}
          >
            {slicerLabel(option)}
          </button>
        ))}
      </div>
      <span className="field-hint">{hint}</span>
    </div>
  );
}

interface SectionProps {
  values: PackFormValues;
  update: Update;
  disabled: boolean;
}

function NozzleCleanFields({
  values,
  update,
  available,
  invalid,
  disabled,
}: SectionProps & { available: boolean; invalid: boolean }) {
  const everyField = (key: 'globalEvery' | 'fromZEvery') => (
    <Field label="Каждый N слой">
      <NumberInput min={1} step={1} value={values[key]} onChange={(value) => update(key, value)} disabled={disabled} />
    </Field>
  );
  return (
    <>
      <Field label="Чистка сопла" hint={available ? undefined : 'Принтер без кода очистки — чистка недоступна'}>
        <select
          className="input"
          value={values.cleanMode}
          onChange={(event) => update('cleanMode', event.target.value as NozzleCleanMode)}
          disabled={!available || disabled}
        >
          <option value="off">Выкл</option>
          <option value="global">Всегда (каждый N-й слой)</option>
          <option value="from-z">С высоты H мм</option>
        </select>
      </Field>
      {values.cleanMode === 'global' && everyField('globalEvery')}
      {values.cleanMode === 'from-z' && (
        <>
          <Field label="Высота H, мм" error={invalid} hint={invalid ? 'Укажите высоту' : undefined}>
            <NumberInput
              min={0}
              value={values.fromZ}
              onChange={(value) => update('fromZ', value)}
              disabled={disabled}
            />
          </Field>
          {everyField('fromZEvery')}
        </>
      )}
    </>
  );
}

function PrintWindowFields({ values, update, disabled }: SectionProps) {
  const hoursValid = inRange(values.windowHours, LIMITS.windowHours);
  const marginValid = inRange(values.marginPct, LIMITS.marginPct);
  return (
    <div className="row2">
      <Field label="Окно, ч" error={!hoursValid} hint={hoursValid ? undefined : rangeHint(LIMITS.windowHours, 'ч')}>
        <NumberInput
          min={LIMITS.windowHours.min}
          max={LIMITS.windowHours.max}
          value={values.windowHours}
          onChange={(value) => update('windowHours', value)}
          disabled={disabled}
        />
      </Field>
      <Field
        label="Запас, %"
        error={!marginValid}
        hint={marginValid ? 'добавляется к оценке слайсера' : rangeHint(LIMITS.marginPct, '%')}
      >
        <NumberInput
          min={LIMITS.marginPct.min}
          max={LIMITS.marginPct.max}
          value={values.marginPct}
          onChange={(value) => update('marginPct', value)}
          disabled={disabled}
        />
      </Field>
    </div>
  );
}

function AdvancedFields({
  values,
  update,
  supportInvalid,
  custom,
  profile,
  disabled,
}: SectionProps & { supportInvalid: boolean; custom: boolean; profile: PrinterProfile | null }) {
  const towerWidthInvalid = !towerWidthValid(values.towerWidth);
  return (
    <>
      <Field
        label="Перекрытие поддержек, мм"
        error={supportInvalid}
        hint={
          supportInvalid
            ? `${rangeHint(LIMITS.supportOverlapMm, 'мм')} или пусто`
            : 'пусто — без ограничений, 0 — поддержки соседей не пересекаются; тело модели никогда не перекрывается'
        }
      >
        <NumberInput
          min={LIMITS.supportOverlapMm.min}
          max={LIMITS.supportOverlapMm.max}
          placeholder="без ограничений"
          value={values.supportOverlap}
          onChange={(value) => update('supportOverlap', value)}
          disabled={disabled}
        />
      </Field>
      <Field
        label="Зазор, мм"
        hint="наименьшее расстояние между соседями вместе с их поддержками и каймой; меньше — плотнее, но поддержки могут слипнуться"
      >
        <NumberInput
          min={LIMITS.clearanceMm.min}
          value={values.clearance}
          onChange={(value) => update('clearance', value)}
          disabled={disabled}
        />
      </Field>
      <Field
        label="Растр, мм"
        hint="шаг сетки поиска раскладки; меньше — точнее и дольше, на плотность обычно не влияет"
      >
        <NumberInput
          min={LIMITS.resolutionMm.min}
          value={values.resolution}
          onChange={(value) => update('resolution', value)}
          disabled={disabled}
        />
      </Field>
      {/* Свой стол несёт отступ в своих полях. */}
      {custom ? null : (
        <Field
          label="Отступ от края, мм"
          hint={profile ? `пусто — дефолт профиля (${profile.edgeMarginMm} мм)` : 'пусто — дефолт профиля'}
        >
          <NumberInput
            min={LIMITS.edgeMarginMm.min}
            placeholder={profile ? String(profile.edgeMarginMm) : ''}
            value={values.edgeMargin}
            onChange={(value) => update('edgeMargin', value)}
            disabled={disabled}
          />
        </Field>
      )}
      <Field
        label="Ширина башни, мм"
        error={towerWidthInvalid}
        hint={
          towerWidthInvalid
            ? `${rangeHint(LIMITS.towerWidthMm, 'мм')} или пусто`
            : 'Узкая башня экономит место; глубину задаёт слайсер'
        }
      >
        <NumberInput
          min={LIMITS.towerWidthMm.min}
          max={LIMITS.towerWidthMm.max}
          placeholder={String(DEFAULT_TOWER_WIDTH_MM)}
          value={values.towerWidth}
          onChange={(value) => update('towerWidth', value)}
          disabled={disabled}
        />
      </Field>
    </>
  );
}
