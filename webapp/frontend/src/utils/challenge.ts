import { serverApi } from '../api/http';
import type { Challenge } from './proof';
export async function uploadProof(signal?: AbortSignal): Promise<{ challengeId: string; nonce: string }> {
  const challenge = await serverApi.post<Challenge>('/challenges', {}, signal);
  if (signal?.aborted) throw new DOMException('Aborted', 'AbortError');
  return new Promise((resolve, reject) => {
    const worker = new Worker(new URL('./proof.worker.ts', import.meta.url), { type: 'module' });
    const cleanup = () => { worker.terminate(); signal?.removeEventListener('abort', abort); clearTimeout(timer); };
    const abort = () => { cleanup(); reject(new DOMException('Aborted', 'AbortError')); };
    const timer = setTimeout(() => { cleanup(); reject(new Error('Проверка заняла слишком много времени')); }, 125_000);
    signal?.addEventListener('abort', abort, { once: true });
    worker.onmessage = (event: MessageEvent<{ nonce?: string; error?: string }>) => {
      cleanup();
      if (event.data.nonce !== undefined) resolve({ challengeId: challenge.id, nonce: event.data.nonce });
      else reject(new Error(event.data.error || 'Ошибка проверки'));
    };
    worker.onerror = () => { cleanup(); reject(new Error('Не удалось выполнить проверку загрузки')); };
    worker.postMessage(challenge);
  });
}
