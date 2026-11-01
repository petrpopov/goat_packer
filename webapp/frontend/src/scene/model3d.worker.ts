import { parseModel } from './model3d';
import { projectTriangles, type ProjectionMask } from './projection';

/** Запрос: имя файла (по расширению выбирается формат) и его содержимое. */
export interface ParseRequest {
  name: string;
  buffer: ArrayBuffer;
}
export type ParseResponse =
  | { ok: true; positions: Float32Array; triangles: number; size: [number, number, number]; projection: ProjectionMask }
  | { ok: false; error: string };

// Разбор 3MF/STL на десятки мегабайт занимает секунды: в основном потоке он замораживал страницу.
// Тип области воркера руками: lib "webworker" рядом с "DOM" в одном tsconfig конфликтует.
const scope = self as unknown as {
  onmessage: ((event: MessageEvent<ParseRequest>) => void) | null;
  postMessage: (message: ParseResponse, transfer: Transferable[]) => void;
};
scope.onmessage = (event: MessageEvent<ParseRequest>) => {
  let response: ParseResponse;
  try {
    const mesh = parseModel(event.data.name, event.data.buffer);
    // Проекция — тоже здесь: в основном потоке растеризация сотен тысяч граней замораживала вкладку.
    response = { ok: true, ...mesh, projection: projectTriangles(mesh.positions, mesh.triangles) };
  } catch (error) {
    response = { ok: false, error: error instanceof Error ? error.message : 'Не удалось прочитать модель' };
  }
  // Буфер треугольников передаётся без копирования.
  scope.postMessage(response, response.ok ? [response.positions.buffer, response.projection.mask.buffer] : []);
};
