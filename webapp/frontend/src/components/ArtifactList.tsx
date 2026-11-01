import type { JobArtifact } from '@/api/types';
import { startDownload, visibleArtifacts } from '@/utils/artifacts';
import { formatBytes } from '@/utils/format';

/** Файлы задачи в карточке (расписание, калибровка, история): имя, размер, «Скачать». */
export default function ArtifactList({ artifacts: all }: { artifacts: JobArtifact[] }) {
  // Порядок — как отдал сервер: здесь важны имена файлов, а не подписи упаковки.
  const shown = new Set(visibleArtifacts(all));
  const artifacts = all.filter((artifact) => shown.has(artifact));
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
            onClick={(event) => {
              event.preventDefault();
              startDownload(artifact);
            }}
          >
            Скачать
          </a>
        </div>
      ))}
    </div>
  );
}
