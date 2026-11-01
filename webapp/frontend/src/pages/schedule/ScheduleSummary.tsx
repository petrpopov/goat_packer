import type { ScheduleInfo } from '@/api/types';
import { formatGrams, formatMinutes } from '@/utils/format';

/** Итог расписания: сводка, таблица работ по партиям и листы мультипластинного 3MF. */
export default function ScheduleSummary({ schedule }: { schedule: ScheduleInfo }) {
  return (
    <div className="stack-sm">
      <p className="section-caption">Сводка</p>
      <div className="kv panel-2">
        <SummaryItem label="Принтер" value={schedule.printer} />
        <SummaryItem label="Всего объектов" value={schedule.totalObjects} mono />
        <SummaryItem label="Филамент" value={`${formatGrams(schedule.totalFilamentGrams)} г`} mono />
        <SummaryItem label="Завершение" value={schedule.completionClock} mono />
      </div>

      <p className="section-caption">Работы</p>
      <div className="table-wrap">
        <table className="table table-compact">
          <thead>
            <tr>
              <th className="num">№</th>
              <th className="num">Партия</th>
              <th>Начало</th>
              <th>Конец</th>
              <th className="num">Слайс, мин</th>
              <th className="num">Подготовка, мин</th>
              <th className="num">Филамент, г</th>
              <th className="num">Накопительно</th>
              <th>Ночь</th>
            </tr>
          </thead>
          <tbody>
            {schedule.jobs.map((entry) => (
              <tr key={entry.sequence}>
                <td className="num mono">{entry.sequence}</td>
                <td className="num mono">{entry.batchSize}</td>
                <td className="mono">{entry.startClock}</td>
                <td className="mono">{entry.endClock}</td>
                <td className="num mono">{formatMinutes(entry.slicerSeconds)}</td>
                <td className="num mono">{formatMinutes(entry.preparationSeconds)}</td>
                <td className="num mono">{formatGrams(entry.filamentGrams)}</td>
                <td className="num mono">{entry.cumulativeObjects}</td>
                <td>
                  {entry.overnight ? (
                    <span className="pill pill-warn">да</span>
                  ) : (
                    <span className="pill pill-muted">нет</span>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      {schedule.plates.length > 0 && (
        <>
          <p className="section-caption">Листы</p>
          <div className="pill-row">
            {schedule.plates.map((plate) => (
              <span className="pill pill-cyan" key={plate.label}>
                {plate.label}
              </span>
            ))}
          </div>
        </>
      )}
    </div>
  );
}

function SummaryItem({ label, value, mono = false }: { label: string; value: string | number; mono?: boolean }) {
  return (
    <div className="kv-item">
      <span className="kv-label">{label}</span>
      <span className={`kv-value${mono ? ' mono' : ''}`}>{value}</span>
    </div>
  );
}
