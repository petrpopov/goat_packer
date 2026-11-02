interface ProgressBarProps {
  percent: number;
  message?: string;
}

export default function ProgressBar({ percent, message }: ProgressBarProps) {
  const clamped = Math.max(0, Math.min(100, percent));
  return (
    <div className="progress">
      <div className="progress-row">
        <span className="progress-percent mono">{clamped}%</span>
        {message ? <span className="progress-message mono">{message}</span> : null}
      </div>
      {/* единственный допустимый inline style — доля полосы (--p, 0..1; рисуется масштабом) */}
      <div className="progress-track">
        <div className="progress-bar" style={{ ['--p' as string]: clamped / 100 }} />
      </div>
    </div>
  );
}
