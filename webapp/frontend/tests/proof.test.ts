import { test, expect } from 'bun:test';
import { createHash } from 'node:crypto';
import { randomId } from '../src/utils/randomId';
import { satisfiesProof, solveProof } from '../src/utils/proof';
test('proof interoperates with SHA256 server format and does not need WebCrypto', () => {
  const c = { id: 'test', salt: 'salt', difficulty: 12, expiresAt: new Date(Date.now() + 5000).toISOString() };
  const nonce = solveProof(c);
  const digest = createHash('sha256').update(`${c.id}:${c.salt}:${nonce}`).digest('hex');
  expect(digest.startsWith('000')).toBe(true);
  expect(satisfiesProof(c, nonce)).toBe(true);
  expect(() => solveProof({ ...c, difficulty: 30 })).toThrow();
  expect(() => solveProof({ ...c, expiresAt: '2000-01-01' })).toThrow();
});
test('UUID fallback is unique and RFC4122 shaped without randomUUID', () => {
  const saved = crypto.randomUUID;
  try {
    Object.defineProperty(crypto, 'randomUUID', { configurable: true, value: undefined });
    const ids = Array.from({ length: 100 }, randomId);
    expect(new Set(ids).size).toBe(100);
    expect(ids.every((id) => /^[a-f0-9]{8}-[a-f0-9]{4}-4[a-f0-9]{3}-[89ab][a-f0-9]{3}-[a-f0-9]{12}$/.test(id))).toBe(true);
  } finally { Object.defineProperty(crypto, 'randomUUID', { configurable: true, value: saved }); }
});
