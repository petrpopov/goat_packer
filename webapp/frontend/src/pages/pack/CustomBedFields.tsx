import type { ProjectBed } from '@/api/types';
import Field from '@/components/Field';
import NumberInput from '@/components/NumberInput';
import { rangeHint, type Range } from '@/utils/forms';
import {
  CUSTOM_BED_MAX_MM,
  CUSTOM_BED_MIN_MM,
  CUSTOM_MARGIN_MAX_MM,
  type CustomBed,
  type CustomBedInput,
} from '@/utils/printers';
import { DEFAULT_CUSTOM_MARGIN_MM } from './packFormValues';

const SIZE: Range = { min: CUSTOM_BED_MIN_MM, max: CUSTOM_BED_MAX_MM };
const MARGIN: Range = { min: 0, max: CUSTOM_MARGIN_MAX_MM };
const SIZE_KEYS = ['width', 'depth', 'height'] as const;

const inBounds = (raw: string, { min, max }: Range) => {
  const value = Number(raw);
  return Number.isFinite(value) && value >= min && value <= max;
};

/** «Моего принтера нет в списке»: размеры стола и отступ; у 3MF неизвестного принтера размеры — из проекта. */
export default function CustomBedFields({
  bed,
  onChange,
  parsed,
  projectBed,
  disabled,
}: {
  bed: CustomBedInput;
  onChange: (bed: CustomBedInput) => void;
  /** Разобранный стол; null — какое-то поле вне границ или отступ съедает стол. */
  parsed: CustomBed | null;
  projectBed: ProjectBed | null;
  disabled: boolean;
}) {
  const sizesComplete = SIZE_KEYS.every((key) => bed[key].trim() !== '' && inBounds(bed[key], SIZE));
  // Размеры из стола проекта не правятся (высота — если она в проекте есть).
  const fixedByProject = (key: keyof CustomBedInput) =>
    projectBed !== null &&
    (key === 'width' || key === 'depth' || (key === 'height' && projectBed.printableHeight != null));

  const field = (key: keyof CustomBedInput, label: string, range: Range, hint?: string) => {
    const raw = bed[key];
    const fixed = fixedByProject(key);
    // Пустое поле не краснеет (его ещё не заполнили); кнопка «Упаковать» без всех размеров неактивна.
    const outOfRange = raw.trim() !== '' && !inBounds(raw, range);
    // Все поля в границах, а стол не собирается — значит, отступ съедает его (как CustomBed.validate).
    const marginTooLarge = key === 'margin' && !outOfRange && !parsed && sizesComplete;
    return (
      <Field
        label={label}
        error={outOfRange || marginTooLarge}
        hint={
          outOfRange
            ? rangeHint(range, 'мм')
            : marginTooLarge
              ? 'отступ больше половины стола'
              : fixed
                ? 'из проекта'
                : hint
        }
      >
        <NumberInput
          min={range.min}
          max={range.max}
          inputMode="decimal"
          placeholder={key === 'margin' ? DEFAULT_CUSTOM_MARGIN_MM : undefined}
          value={raw}
          onChange={(value) => onChange({ ...bed, [key]: value })}
          readOnly={fixed}
          disabled={disabled}
        />
      </Field>
    );
  };

  return (
    <div className="nested custom-bed">
      <div className="row3">
        {field('width', 'Ширина, мм', SIZE)}
        {field('depth', 'Глубина, мм', SIZE)}
        {field('height', 'Высота, мм', SIZE)}
      </div>
      {field('margin', 'Отступ от края, мм', MARGIN, 'линия прочистки неизвестна — держим запас')}
    </div>
  );
}
