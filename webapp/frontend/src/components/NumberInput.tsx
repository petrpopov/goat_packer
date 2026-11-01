import type { InputHTMLAttributes } from 'react';

type NumberInputProps = Omit<InputHTMLAttributes<HTMLInputElement>, 'type' | 'value' | 'onChange'> & {
  /** Строка поля как есть: пустое значение и промежуточный ввод («0.») не теряются. */
  value: string;
  onChange: (value: string) => void;
};

/** Числовое поле формы: шаг по умолчанию любой, значение — строкой (проверка — у формы). */
export default function NumberInput({ value, onChange, step = 'any', className = 'input', ...rest }: NumberInputProps) {
  return (
    <input
      {...rest}
      className={className}
      type="number"
      step={step}
      value={value}
      onChange={(event) => onChange(event.target.value)}
    />
  );
}
