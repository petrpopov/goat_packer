import Field from './Field';

interface AmsCorrectionFieldsProps {
  available: boolean;
  on: boolean;
  onToggle: (on: boolean) => void;
  seconds: string;
  onSecondsChange: (value: string) => void;
  invalid?: boolean;
  disabled?: boolean;
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
          Коррекция времени смены прутка (AMS)
        </label>
        <span className="field-hint">
          {available ? 'слайсер занижает время смены — учесть реальное' : 'для этого принтера коррекция не нужна'}
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
    </>
  );
}
