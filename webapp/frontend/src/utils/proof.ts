import { sha256 } from '@noble/hashes/sha2.js';
export type Challenge = { id: string; salt: string; difficulty: number; expiresAt: string };
export function satisfiesProof(challenge: Challenge, nonce: string): boolean {
  const digest = sha256(new TextEncoder().encode(`${challenge.id}:${challenge.salt}:${nonce}`));
  let bits = 0;
  for (const byte of digest) {
    if (byte === 0) bits += 8;
    else { bits += Math.clz32(byte) - 24; break; }
  }
  return bits >= challenge.difficulty;
}
export function solveProof(challenge: Challenge): string {
  if (!Number.isInteger(challenge.difficulty) || challenge.difficulty < 1 || challenge.difficulty > 22)
    throw new Error('Неподдерживаемая сложность проверки');
  const end = Math.min(Date.parse(challenge.expiresAt), Date.now() + 120_000);
  if (!Number.isFinite(end)) throw new Error('Некорректный срок проверки');
  for (let n = 0; ; n++) {
    if (n % 1024 === 0 && Date.now() >= end) throw new Error('Проверка истекла, повторите загрузку');
    if (satisfiesProof(challenge, String(n))) return String(n);
  }
}
