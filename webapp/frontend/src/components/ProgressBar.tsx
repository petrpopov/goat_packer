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
      {/* единственный допустимый inline style — динамическая ширина полосы */}
      <div className="progress-track">
        <div className="progress-bar" style={{ width: `${clamped}%` }} />
      </div>
    </div>
  );
}
