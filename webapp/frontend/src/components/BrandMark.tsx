/**
 * Знак Goat Packer: изометрический куб, из-за которого выглядывает коза (глаза-точки, рога назад,
 * ухо, бородка). Цвета — токены темы; public/favicon.svg — тот же рисунок с фиксированными цветами.
 * Голова рисуется раньше куба: куб непрозрачный (подложка --bg-chrome под --accent-soft) и прячет её край.
 */
export default function BrandMark({ size = 26 }: { size?: number }) {
  const stroke = {
    stroke: 'var(--accent-fill)',
    strokeWidth: 2.2,
    strokeLinecap: 'round',
    strokeLinejoin: 'round',
  } as const;
  const cube = 'M11.5 9 19.5 13.5v9L11.5 27l-8-4.5v-9Z';
  return (
    <svg viewBox="0 0 34 30" width={(size * 34) / 30} height={size} aria-hidden="true">
      <g transform="translate(0.6 0)">
        <path d="M20.2 8.2c-.2-2.6 1.2-4.8 3.6-5.8M25.2 8c.4-2.4 2-4.2 4.4-4.8" fill="none" {...stroke} />
        <path d="M28.4 11.6c1.8.3 3.3-.2 4.1-1.3-1.3-.9-3-1-4.4-.2" fill="var(--panel)" {...stroke} strokeWidth={1.7} />
        <path
          d="M17.4 12c0-2.6 2.5-4.3 5.6-4.3s5.6 1.7 5.6 4.3v4.4c0 2.9-2.3 5.1-4.4 5.4L19 21.6 17.4 14.5Z"
          fill="var(--panel)"
        />
        <path
          d="M17.4 14.5V12c0-2.6 2.5-4.3 5.6-4.3s5.6 1.7 5.6 4.3v4.4c0 2.9-2.3 5.1-4.4 5.4"
          fill="none"
          {...stroke}
        />
        <circle cx="21.4" cy="13.4" r="1.7" fill="var(--accent)" />
        <circle cx="25.6" cy="13.4" r="1.7" fill="var(--accent)" />
        <path d="M24.6 21.8 24 25.2" fill="none" {...stroke} />
      </g>
      <path d={cube} fill="var(--chrome)" />
      <path d={cube} fill="var(--accent-soft)" {...stroke} />
      <path d="M3.5 13.5l8 4.5 8-4.5M11.5 18v9" fill="none" {...stroke} />
    </svg>
  );
}
