import { useEffect, useState } from 'react';
import type { JobStage } from '@/api/types';
import { looksLikeJson, packSteps, stageElapsed, stageTimeLine } from '@/utils/progress';
import ProgressBar from './ProgressBar';

interface PackProgressProps {
  percent: number;
  stage: JobStage;
  /** Русское описание этапа от бэкенда. */
  message: string | null;
  /** Когда получен ответ со [stage] (Date.now() клиента). */
  receivedAtMs?: number;
}

const TICK_MS = 1000;

/**
 * Прогресс упаковки: шкала, строка статуса с временем этапа и списком этапов. Время этапа
 * досчитывается локально раз в секунду между опросами, процент — только с сервера.
 */
export default function PackProgress({ percent, stage, message, receivedAtMs }: PackProgressProps) {
  const [now, setNow] = useState(receivedAtMs ?? 0);
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), TICK_MS);
    return () => window.clearInterval(timer);
  }, []);

  const elapsed = stageElapsed(stage, receivedAtMs, Math.max(now, receivedAtMs ?? 0));
  const steps = packSteps(stage);
  const detail = stage.detail && !looksLikeJson(stage.detail) ? stage.detail : null;
  const status = message && !looksLikeJson(message) ? message : null;

  return (
    <div className="pack-progress">
      <ProgressBar percent={percent} />
      <div className="stage-status">
        {status ? <span className="stage-status-text">{status}</span> : null}
        <span className="stage-status-time mono">{stageTimeLine(elapsed, stage.expectedSeconds)}</span>
      </div>
      {detail ? <div className="stage-detail mono">{detail}</div> : null}
      <ol className="stage-list">
        {steps.map((step) => (
          <li key={step.id} className={`stage-step stage-step-${step.state}`}>
            <span className="stage-marker" aria-hidden="true">
              {step.state === 'done' ? '✓' : step.state === 'current' ? '●' : '○'}
            </span>
            <span className="stage-label">{step.label}</span>
          </li>
        ))}
      </ol>
    </div>
  );
}
