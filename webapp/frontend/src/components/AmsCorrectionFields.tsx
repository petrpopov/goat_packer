import Field from './Field';

interface AmsCorrectionFieldsProps {
  available: boolean;
  on: boolean;
  onToggle: (on: boolean) => void;
  seconds: string;
  onSecondsChange: (value: string) => void;
  invalid?: boolean;
  disabled?: boolean;
  /** Открыть панель калибровки (только в упаковке). */
  onCalibrate?: () => void;
}

/** Галочка коррекции смены прутка AMS + длительность смены, с (по умолчанию — из профиля). */
export default function AmsCorrectionFields({
  available,
  on,
  onToggle,
  seconds,
  onSecondsChange,
  invalid = false,
  disabled = false,
  onCalibrate,
}: AmsCorrectionFieldsProps) {
  return (
    <>
      <div className="field">
        <label className="check">
          <input
            type="checkbox"
            checked={on}
            onChange={(event) => onToggle(event.target.checked)}
            disabled={!available || disabled}
          />
          Учитывать реальную смену прутка
        </label>
        <span className="field-hint">
          {available ? 'слайсер занижает время смены AMS — окно считается точнее' : 'для этого принтера коррекция не нужна'}
        </span>
      </div>
      {on && (
        <Field label="Смена прутка, с" error={invalid} hint={invalid ? '0–600 с' : 'пусто — значение профиля'}>
          <input
            className="input"
            type="number"
            min={0}
            max={600}
            step="any"
            value={seconds}
            onChange={(event) => onSecondsChange(event.target.value)}
            disabled={disabled}
          />
        </Field>
      )}
      {on && onCalibrate && (
        <button type="button" className="link" style={{ justifySelf: 'start' }} onClick={onCalibrate}>
          Откалибровать…
        </button>
      )}
    </>
  );
}
