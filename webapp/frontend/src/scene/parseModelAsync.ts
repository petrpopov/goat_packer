import type { ModelMesh } from './model3d';
import type { ParseRequest, ParseResponse } from './model3d.worker';

/**
 * Файлы крупнее предпросмотр не строят: Float32Array треугольников и буферы WebGL для такой модели
 * съедают сотни мегабайт вкладки, а на сервер файл всё равно уходит целиком.
 */
export const PREVIEW_MAX_BYTES = 80 * 1024 * 1024;

/** Разбор модели в отдельном воркере; cancel() останавливает его (новый файл, уход со страницы). */
export function parseModelInWorker(name: string, buffer: ArrayBuffer): { promise: Promise<ModelMesh>; cancel: () => void } {
  const worker = new Worker(new URL('./model3d.worker.ts', import.meta.url), { type: 'module' });
  const promise = new Promise<ModelMesh>((resolve, reject) => {
    worker.onmessage = (event: MessageEvent<ParseResponse>) => {
      worker.terminate();
      const data = event.data;
      if (data.ok) resolve({ positions: data.positions, triangles: data.triangles, size: data.size, projection: data.projection });
      else reject(new Error(data.error));
    };
    worker.onerror = (event) => {
      worker.terminate();
      reject(new Error(event.message || 'Не удалось прочитать модель'));
    };
  });
  // Буфер файла передаётся воркеру без копирования: в основном потоке он больше не нужен.
  const request: ParseRequest = { name, buffer };
  worker.postMessage(request, [buffer]);
  return { promise, cancel: () => worker.terminate() };
}
