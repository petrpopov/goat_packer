import { useEffect, useState } from 'react';
import type { JobStage } from '@/api/types';
import { looksLikeJson, stageElapsed, stageSegments, stageTimeLine } from '@/utils/progress';

interface PackProgressProps {
  percent: number;
  stage: JobStage;
  /** Русское описание этапа от бэкенда. */
  message: string | null;
  /** Когда получен ответ со [stage] (Date.now() клиента). */
  receivedAtMs?: number;
}

const TICK_MS = 1000;
/** Этапы, в которые пакер заходит не всегда: пока не наступили — «если понадобится». */
const OPTIONAL = new Set(['gcode_check', 'repacking', 'window_slice']);

/**
 * Одна шкала из сегментов этапов (ширина = полоса процента этапа) + строка статуса + этапы.
 * Время этапа досчитывается локально раз в секунду между опросами, процент — только с сервера.
 */
export default function PackProgress({ percent, stage, message, receivedAtMs }: PackProgressProps) {
  const [now, setNow] = useState(receivedAtMs ?? 0);
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), TICK_MS);
    return () => window.clearInterval(timer);
  }, []);
  const elapsed = stageElapsed(stage, receivedAtMs, Math.max(now, receivedAtMs ?? 0));
  const segments = stageSegments(stage, percent);
  const detail = stage.detail && !looksLikeJson(stage.detail) ? stage.detail : null;
  const current = segments.find((s) => s.state === 'current');
  const status = message && !looksLikeJson(message) ? message : current?.label;

  return (
    <div className="pack-progress" role="group" aria-label="Ход упаковки">
      <div className="stage-status">
        <span className="stage-status-text">{status}</span>
        <span className="stage-status-time mono">
          {Math.round(percent)}% · {stageTimeLine(elapsed, stage.expectedSeconds)}
        </span>
      </div>
      <div
        className="stage-bar"
        role="progressbar"
        aria-label="Ход упаковки"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={Math.round(percent)}
        aria-valuetext={`${current?.label ?? ''} · ${Math.round(percent)}%`}
      >
        {segments.map((s) => (
          <span
            key={s.id}
            className={s.state}
            style={{ flex: s.weight, ['--fill' as string]: `${Math.round(s.fill * 100)}%` }}
          />
        ))}
      </div>
      {detail ? <div className="stage-detail mono">{detail}</div> : null}
      <ul className="stage-chips">
        {segments.map((s) => {
          const optional = OPTIONAL.has(s.id) && s.state === 'pending';
          return (
            <li key={s.id} className={optional ? 'optional' : s.state}>
              <i aria-hidden="true" />
              {s.label}
              {s.state === 'skipped' ? ' · не понадобилась' : optional ? ' · если понадобится' : ''}
            </li>
          );
        })}
      </ul>
    </div>
  );
}
