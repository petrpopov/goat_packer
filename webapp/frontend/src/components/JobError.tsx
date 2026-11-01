import { splitJobError } from '@/utils/jobs';

/**
 * Ошибка задачи: человекопонятный заголовок крупно, сырой хвост журнала ниже (бэкенд склеивает их
 * через ERROR_DETAILS_SEPARATOR; без разделителя — только заголовок). collapsible — хвост свёрнут.
 */
export default function JobError({ error, collapsible = false }: { error: string; collapsible?: boolean }) {
  const { headline, details } = splitJobError(error);
  const log = details ? <pre className="error-details">{details}</pre> : null;
  return (
    <>
      <div className="error-block">{headline}</div>
      {log && collapsible ? (
        <details className="params">
          <summary className="params-summary">Подробности журнала</summary>
          {log}
        </details>
      ) : (
        log
      )}
    </>
  );
}
