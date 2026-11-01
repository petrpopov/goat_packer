import type { ReactNode } from 'react';

/** Полоса по низу строки: progress — доля 0..100 (только растёт), waiting — очередь, ok/fail — итог. */
export type StripLine = { kind: 'off' | 'waiting' | 'ok' | 'fail' } | { kind: 'progress'; percent: number };

interface StatusStripProps {
  title: ReactNode;
  detail: ReactNode;
  line: StripLine;
  /** Подпись полосы для читалок экрана (aria-valuetext). */
  lineLabel?: string;
  actions: ReactNode;
}

/**
 * Строка статуса над сценой: фиксированная высота, обе строки — в одну строку с многоточием.
 * Сцена под ней всегда на одной высоте: подробности (этапы, файлы, ошибка) открываются поверх сцены.
 */
export default function StatusStrip({ title, detail, line, lineLabel, actions }: StatusStripProps) {
  const percent = line.kind === 'progress' ? Math.max(0, Math.min(100, line.percent)) : null;
  return (
    <div className="status-strip">
      <div className="lines">
        <h1 className="l1">{title}</h1>
        <p className="l2">{detail}</p>
      </div>
      <div className="strip-actions">{actions}</div>
      <div
        className={`strip-line ${line.kind === 'progress' ? '' : line.kind}`}
        role={percent !== null || line.kind === 'waiting' ? 'progressbar' : undefined}
        aria-label={percent !== null || line.kind === 'waiting' ? 'Ход задачи' : undefined}
        aria-valuemin={percent !== null ? 0 : undefined}
        aria-valuemax={percent !== null ? 100 : undefined}
        aria-valuenow={percent !== null ? Math.round(percent) : undefined}
        aria-valuetext={lineLabel}
      >
        <i style={percent !== null ? { ['--p' as string]: `${percent}%` } : undefined} />
      </div>
    </div>
  );
}
