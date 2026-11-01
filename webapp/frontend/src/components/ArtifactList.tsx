import { serverApi, serverMode } from '@/api/http';
import { pushToast } from '@/store/useToastStore';
import type { JobArtifact } from '@/api/types';
import { formatBytes } from '@/utils/format';

interface ArtifactListProps {
  artifacts: JobArtifact[];
}

/** Служебные файлы: состояние упаковки нужно только кнопке «Нарезать стол», не пользователю. */
const HIDDEN = new Set(['pack_state.json']);

export default function ArtifactList({ artifacts: all }: ArtifactListProps) {
  const artifacts = all.filter((artifact) => !HIDDEN.has(artifact.name));
  if (artifacts.length === 0) return null;

  return (
    <div className="artifact-list">
      <p className="section-caption">Артефакты</p>
      {artifacts.map((artifact) => (
        <div className="artifact-row" key={artifact.name}>
          <span className="artifact-name mono">{artifact.name}</span>
          <span className="artifact-size mono">{formatBytes(artifact.sizeBytes)}</span>
          <a
            className="btn"
            href={artifact.downloadUrl}
            download={artifact.name}
            onClick={
              serverMode
                ? (event) => {
                    event.preventDefault();
                    void serverApi
                      .post<{ url: string }>(artifact.downloadUrl.replace('/api/v2', ''))
                      .then((r) => {
                        location.href = r.url;
                      })
                      .catch((e) => pushToast(e.message, 'error'));
                  }
                : undefined
            }
          >
            Скачать
          </a>
        </div>
      ))}
    </div>
  );
}
