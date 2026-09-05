import type { ReactNode } from 'react';

interface FieldProps {
  label: string;
  hint?: string;
  error?: boolean;
  children: ReactNode;
}

/** label + control (+ подсказка); label обёрткой даёт клик-в-фокус. */
export default function Field({ label, hint, error = false, children }: FieldProps) {
  return (
    <label className={`field${error ? ' field-error' : ''}`}>
      <span className="field-label">{label}</span>
      {children}
      {hint ? <span className="field-hint">{hint}</span> : null}
    </label>
  );
}
