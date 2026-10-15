export function numberInRange(value: string, min: number, max: number, integer = false): boolean {
  if (value.trim() === '') return false;
  const n = Number(value);
  return Number.isFinite(n) && n >= min && n <= max && (!integer || Number.isInteger(n));
}

export function clockSeconds(value: string): number | null {
  const match = /^(\d{1,2}):([0-5]\d)$/.exec(value.trim());
  if (!match || Number(match[1]) > 47) return null;
  return Number(match[1]) * 3600 + Number(match[2]) * 60;
}

export function validTimeWindow(day: string, night: string, end: string): boolean {
  const d = clockSeconds(day),
    n = clockSeconds(night),
    e = clockSeconds(end);
  return d !== null && n !== null && e !== null && d < n && n < e;
}

export function buildNozzleClean(
  mode: 'off' | 'global' | 'from-z',
  fromZ: string,
  globalEvery: string,
  fromZEvery: string
): string {
  if (mode === 'off') return '';
  const every = mode === 'global' ? globalEvery : fromZEvery;
  if (!numberInRange(every, 1, Number.MAX_SAFE_INTEGER, true))
    throw new Error('Период очистки должен быть целым числом ≥ 1');
  if (mode === 'from-z' && !numberInRange(fromZ, Number.MIN_VALUE, 100000))
    throw new Error('Высота очистки должна быть положительным числом');
  // `global` without every means 10 in the CLI, not 1. Always send the actual UI value.
  return `${mode === 'global' ? 'global' : `from-z=${Number(fromZ)}`},every=${Number(every)}`;
}
