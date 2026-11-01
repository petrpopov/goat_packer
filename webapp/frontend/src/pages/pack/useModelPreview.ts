import { useEffect, useState } from 'react';
import type { ModelMesh } from '@/scene/model3d';
import { parseModelInWorker, PREVIEW_MAX_BYTES } from '@/scene/parseModelAsync';
import { pushToast } from '@/store/useToastStore';
import { errorMessage } from '@/utils/jobs';

/**
 * Предпросмотр модели в сцене разбирается в браузере из того же файла, что уйдёт на сервер —
 * в воркере, чтобы разбор крупного 3MF не замораживал страницу. Слишком большой файл не разбирается.
 */
export function useModelPreview(file: File | null): { model: ModelMesh | null; parsing: boolean; tooLarge: boolean } {
  const [model, setModel] = useState<ModelMesh | null>(null);
  const [parsing, setParsing] = useState(false);
  const tooLarge = file !== null && file.size > PREVIEW_MAX_BYTES;

  useEffect(() => {
    let cancelled = false;
    let cancelParse: (() => void) | null = null;
    setModel(null);
    setParsing(false);
    if (!file || file.size > PREVIEW_MAX_BYTES) return;
    setParsing(true);
    file
      .arrayBuffer()
      .then((buffer) => {
        if (cancelled) return null;
        const parse = parseModelInWorker(file.name, buffer);
        cancelParse = parse.cancel;
        return parse.promise;
      })
      .then((mesh) => {
        if (!cancelled && mesh) setModel(mesh);
      })
      .catch((error: unknown) => {
        if (!cancelled) pushToast(errorMessage(error, 'Не удалось прочитать модель'), 'error');
      })
      .finally(() => {
        if (!cancelled) setParsing(false);
      });
    return () => {
      cancelled = true;
      cancelParse?.();
    };
  }, [file]);

  return { model, parsing, tooLarge };
}
