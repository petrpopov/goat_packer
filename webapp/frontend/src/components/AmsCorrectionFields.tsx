import { inRange, LIMITS, rangeHint } from '@/utils/forms';
import Field from './Field';
import NumberInput from './NumberInput';

interface AmsCorrectionFieldsProps {
  available: boolean;
  on: boolean;
  onToggle: (on: boolean) => void;
  seconds: string;
  onSecondsChange: (value: string) => void;
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
  disabled = false,
  onCalibrate,
}: AmsCorrectionFieldsProps) {
  // Пусто — значение профиля, это не ошибка.
  const invalid = seconds.trim() !== '' && !inRange(seconds, LIMITS.changeSeconds);
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
          {/* Про окно — только в упаковке (там коррекция работает лишь внутри окна печати). */}
          {available
            ? onCalibrate
              ? 'слайсер занижает время смены AMS — окно считается точнее'
              : 'слайсер занижает время смены AMS — учесть реальное'
            : 'для этого принтера коррекция не нужна'}
        </span>
      </div>
      {on && (
        <Field
          label="Смена прутка, с"
          error={invalid}
          hint={invalid ? rangeHint(LIMITS.changeSeconds, 'с') : 'пусто — значение профиля'}
        >
          <NumberInput
            min={LIMITS.changeSeconds.min}
            max={LIMITS.changeSeconds.max}
            value={seconds}
            onChange={onSecondsChange}
            disabled={disabled}
          />
        </Field>
      )}
      {on && onCalibrate && (
        <button type="button" className="link link-start" onClick={onCalibrate}>
          Откалибровать…
        </button>
      )}
    </>
  );
}
