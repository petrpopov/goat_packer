import type { ReactNode } from 'react';

/** «Назад · N · Далее» для списков, которые бэкенд отдаёт страницами по pageSize. */
export default function Pager({
  page,
  total,
  pageSize,
  onChange,
  label = page + 1,
}: {
  page: number;
  total: number;
  pageSize: number;
  onChange: (page: number) => void;
  label?: ReactNode;
}) {
  return (
    <div className="form-actions">
      <button className="btn" disabled={page === 0} onClick={() => onChange(page - 1)}>
        Назад
      </button>
      <span>{label}</span>
      <button className="btn" disabled={(page + 1) * pageSize >= total} onClick={() => onChange(page + 1)}>
        Далее
      </button>
    </div>
  );
}
