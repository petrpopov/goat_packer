import { useState } from 'react';
import { pushToast } from '@/store/useToastStore';
import { useCalibrationStore } from '@/store/useCalibrationStore';
import { computeChangeSeconds, parseCalibrationJson, storeCalibration } from '@/utils/calibration';
import type { CalibrationValues } from '@/utils/calibration';
import Field from './Field';

interface AmsCalculatorProps {
  /** Принтер, для которого сохраняется результат; null — сохранить нельзя. */
  profileId: string | null;
  profileName?: string;
  /** Значения calibration.json (из задачи или сохранённые ранее); null — ввести вручную. */
  initial: CalibrationValues | null;
}

const toText = (value: number | undefined) => (value === undefined || !Number.isFinite(value) ? '' : String(value));

/**
 * Расчёт полной длительности смены по четырём записанным временам. Значения calibration.json
 * подставляются из задачи, но редактируемы: пользователь может вернуться через день, без задачи,
 * и ввести их вручную или вставить файл целиком. Родитель задаёт key, чтобы новые значения
 * задачи сбрасывали форму.
 */
export default function AmsCalculator({ profileId, profileName, initial }: AmsCalculatorProps) {
  const saveChangeSeconds = useCalibrationStore((state) => state.saveChangeSeconds);
  const [clocks, setClocks] = useState({ aStart: '', aFinish: '', bStart: '', bFinish: '' });
  const [estimateA, setEstimateA] = useState(toText(initial?.estimateASeconds));
  const [estimateB, setEstimateB] = useState(toText(initial?.estimateBSeconds));
  const [changes, setChanges] = useState(toText(initial?.changes));
  const [slicerChange, setSlicerChange] = useState(toText(initial?.slicerChangeSeconds));
  const [pasted, setPasted] = useState('');

  const num = (value: string) => (value.trim() === '' ? NaN : Number(value));
  const values: CalibrationValues = {
    estimateASeconds: num(estimateA),
    estimateBSeconds: num(estimateB),
    changes: num(changes),
    slicerChangeSeconds: num(slicerChange),
  };
  const outcome = computeChangeSeconds(clocks, values);
  const allClocks = Object.values(clocks).every((value) => value.trim() !== '');
  // Поле коррекции на бэкенде принимает 0–600 с.
  const storable = outcome.ok && outcome.realSeconds >= 0 && outcome.realSeconds <= 600;

  const applyPasted = () => {
    const parsed = parseCalibrationJson(pasted);
    if (!parsed) {
      pushToast('Это не calibration.json: нужны estimate_a_s, estimate_b_s, changes_sliced, slicer_change_s', 'error');
      return;
    }
    setEstimateA(String(parsed.estimateASeconds));
    setEstimateB(String(parsed.estimateBSeconds));
    setChanges(String(parsed.changes));
    setSlicerChange(String(parsed.slicerChangeSeconds));
    setPasted('');
  };

  const applyCorrection = () => {
    if (!profileId || !outcome.ok) return;
    saveChangeSeconds(profileId, outcome.realSeconds);
    storeCalibration(profileId, values);
    pushToast(`Смена прутка ${outcome.realSeconds} с сохранена для «${profileName ?? profileId}»`, 'success');
  };

  const clockField = (key: keyof typeof clocks, label: string) => (
    <Field label={label}>
      <input
        className="input"
        type="time"
        value={clocks[key]}
        onChange={(event) => setClocks((current) => ({ ...current, [key]: event.target.value }))}
      />
    </Field>
  );

  return (
    <section className="card stack">
      <div>
        <h2 className="job-card-title">Расчёт после печати</h2>
        <p className="field-hint">
          Время нажатия «Печать» и уведомления «Печать завершена», ЧЧ:ММ; конец раньше начала — после полуночи.
        </p>
      </div>
      <div className="form-grid">
        {clockField('aStart', 'A: начало')}
        {clockField('aFinish', 'A: конец')}
        {clockField('bStart', 'B: начало')}
        {clockField('bFinish', 'B: конец')}
      </div>

      <details className="params" open={initial === null}>
        <summary className="params-summary">Значения из calibration.json</summary>
        <div className="form-grid">
          <Field label="Оценка A, с">
            <input
              className="input"
              type="number"
              step="any"
              value={estimateA}
              onChange={(e) => setEstimateA(e.target.value)}
            />
          </Field>
          <Field label="Оценка B, с">
            <input
              className="input"
              type="number"
              step="any"
              value={estimateB}
              onChange={(e) => setEstimateB(e.target.value)}
            />
          </Field>
          <Field label="Смен в B">
            <input
              className="input"
              type="number"
              min={1}
              step={1}
              value={changes}
              onChange={(e) => setChanges(e.target.value)}
            />
          </Field>
          <Field label="Смена в слайсере, с">
            <input
              className="input"
              type="number"
              step="any"
              value={slicerChange}
              onChange={(e) => setSlicerChange(e.target.value)}
            />
          </Field>
        </div>
        <div className="stack-sm">
          <textarea
            className="input mono"
            rows={3}
            placeholder="…или вставьте сюда содержимое calibration.json"
            value={pasted}
            onChange={(e) => setPasted(e.target.value)}
          />
          <div>
            <button type="button" className="btn" disabled={pasted.trim() === ''} onClick={applyPasted}>
              Подставить из calibration.json
            </button>
          </div>
        </div>
      </details>

      {outcome.ok ? (
        <div className="stack-sm">
          <p className="result-title">Полная длительность одной смены: {outcome.realSeconds} с</p>
          <p className="time-summary">лишнее сверх слайсера: {outcome.extraSeconds} с на смену</p>
          {outcome.warning ? <div className="warn-block">{outcome.warning}</div> : null}
          <div className="form-actions">
            <button
              type="button"
              className="btn btn-primary"
              disabled={!profileId || !storable}
              onClick={applyCorrection}
            >
              Использовать как коррекцию
            </button>
            {!profileId ? <span className="field-hint">выберите принтер, чтобы сохранить</span> : null}
            {profileId && !storable ? <span className="field-hint">коррекция принимает 0–600 с</span> : null}
          </div>
        </div>
      ) : allClocks ? (
        <div className="error-block">{outcome.error}</div>
      ) : null}
    </section>
  );
}
