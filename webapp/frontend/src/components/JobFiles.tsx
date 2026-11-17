import type { JobArtifact } from '@/api/types';
import { describeArtifact, isPlateFile, startDownload, visibleArtifacts } from '@/utils/artifacts';
import { formatBytes, formatShortDateTime } from '@/utils/format';
import Icon from './Icon';

/** Файлы задачи в панели «Инфо»: подписи по-человечески, стол первым, срок хранения внизу. */
export default function JobFiles({
  artifacts,
  expiresAt,
}: {
  artifacts: JobArtifact[];
  expiresAt?: string | null;
}) {
  const files = visibleArtifacts(artifacts);
  return (
    <>
      {files.length ? (
        <div className="file-list">
          {files.map((artifact) => (
            <a
              key={artifact.downloadUrl}
              href={artifact.downloadUrl}
              download={artifact.name}
              onClick={(event) => {
                event.preventDefault();
                startDownload(artifact);
              }}
            >
              <Icon name={isPlateFile(artifact) ? 'download' : 'file'} />
              <span className="name">{describeArtifact(artifact.name).label}</span>
              <small>{formatBytes(artifact.sizeBytes)}</small>
            </a>
          ))}
        </div>
      ) : (
        <p className="hint">Файлов нет.</p>
      )}
      {expiresAt ? <p className="hint">Файлы хранятся до {formatShortDateTime(expiresAt)}.</p> : null}
    </>
  );
}
