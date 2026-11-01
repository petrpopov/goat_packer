import { downloadArtifact } from '@/api/services';
import type { JobArtifact } from '@/api/types';
import { pushToast } from '@/store/useToastStore';
import { formatBytes } from '@/utils/format';
import Icon from './Icon';

/** Служебные файлы: состояние упаковки нужно только кнопке «Нарезать стол», не пользователю. */
const HIDDEN = new Set(['pack_state.json']);

/** Человеческие подписи файлов упаковки; результат — первым. */
function describe(name: string): { label: string; order: number } {
  if (name.endsWith('.3mf')) return { label: `${name} — стол для слайсера`, order: 0 };
  if (name === 'log.txt') return { label: 'Журнал упаковки', order: 1 };
  if (name.endsWith('verification.json')) return { label: 'Отчёт проверки (JSON)', order: 2 };
  if (name === 'placements.json') return { label: 'Координаты объектов (JSON)', order: 3 };
  return { label: name, order: 4 };
}

export function visibleArtifacts(artifacts: JobArtifact[]): JobArtifact[] {
  return artifacts
    .filter((artifact) => !HIDDEN.has(artifact.name))
    .sort((a, b) => describe(a.name).order - describe(b.name).order);
}

export function startDownload(artifact: JobArtifact): void {
  downloadArtifact(artifact).catch((error: unknown) =>
    pushToast(error instanceof Error ? error.message : 'Не удалось скачать файл', 'error')
  );
}

export default function JobFiles({ artifacts, expiresAt }: { artifacts: JobArtifact[]; expiresAt?: string | null }) {
  const files = visibleArtifacts(artifacts);
  return (
    <>
      {files.length ? (
        <div className="file-list">
          {files.map((artifact) => (
            <a
              key={artifact.name}
              href={artifact.downloadUrl}
              download={artifact.name}
              onClick={(event) => {
                event.preventDefault();
                startDownload(artifact);
              }}
            >
              <Icon name={artifact.name.endsWith('.3mf') ? 'download' : 'file'} />
              {describe(artifact.name).label}
              <small>{formatBytes(artifact.sizeBytes)}</small>
            </a>
          ))}
        </div>
      ) : (
        <p className="hint">Файлов нет.</p>
      )}
      {expiresAt ? (
        <p className="hint">
          Файлы хранятся до{' '}
          {new Date(expiresAt).toLocaleString('ru-RU', {
            day: '2-digit',
            month: '2-digit',
            hour: '2-digit',
            minute: '2-digit',
          })}
          .
        </p>
      ) : null}
    </>
  );
}
