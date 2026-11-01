/** Файлы задачи: что показывать пользователю, как подписать и как скачать. */

import { downloadArtifact } from '../api/services';
import type { JobArtifact } from '../api/types';
import { pushToast } from '../store/useToastStore';
import { errorMessage } from './jobs';

/** Служебные файлы: состояние упаковки нужно только проверке нарезкой, не пользователю. */
const HIDDEN = new Set(['pack_state.json']);

export const PACK_LOG = 'log.txt';
export const isPlateFile = (artifact: JobArtifact) => artifact.name.endsWith('.3mf');

/** Человеческие подписи файлов упаковки; результат — первым. */
export function describeArtifact(name: string): { label: string; order: number } {
  if (name.endsWith('.3mf')) return { label: `${name} — стол для слайсера`, order: 0 };
  if (name === PACK_LOG) return { label: 'Журнал упаковки', order: 1 };
  if (name.endsWith('verification.json')) return { label: 'Отчёт проверки (JSON)', order: 2 };
  if (name === 'placements.json') return { label: 'Координаты объектов (JSON)', order: 3 };
  return { label: name, order: 4 };
}

export function visibleArtifacts(artifacts: JobArtifact[]): JobArtifact[] {
  return artifacts
    .filter((artifact) => !HIDDEN.has(artifact.name))
    .sort((a, b) => describeArtifact(a.name).order - describeArtifact(b.name).order);
}

/** Скачать файл; ошибка (например, истёкшая подписанная ссылка) — тостом. */
export function startDownload(artifact: JobArtifact): void {
  downloadArtifact(artifact).catch((error: unknown) =>
    pushToast(errorMessage(error, 'Не удалось скачать файл'), 'error')
  );
}
