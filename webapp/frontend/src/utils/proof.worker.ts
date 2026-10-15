import { solveProof, type Challenge } from './proof';
self.onmessage = (event: MessageEvent<Challenge>) => {
  try { self.postMessage({ nonce: solveProof(event.data) }); }
  catch (error) { self.postMessage({ error: error instanceof Error ? error.message : 'Ошибка проверки' }); }
};
