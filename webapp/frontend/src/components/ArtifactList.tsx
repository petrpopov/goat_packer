import { serverApi, serverMode } from '@/api/http';
import { pushToast } from '@/store/useToastStore';
import type { JobArtifact } from '@/api/types';
import { formatBytes } from '@/utils/format';

interface ArtifactListProps {
  artifacts: JobArtifact[];
}

export default function ArtifactList({ artifacts }: ArtifactListProps) {
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
