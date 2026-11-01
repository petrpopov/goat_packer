import { describe, expect, test } from 'bun:test';
import { parseRoute, routePath } from '../src/utils/routes';

describe('parseRoute', () => {
  test('root and unknown paths open packing', () => {
    expect(parseRoute('/')).toEqual({ name: 'pack' });
    expect(parseRoute('/nope/x')).toEqual({ name: 'pack' });
  });
  test('sections', () => {
    expect(parseRoute('/pack')).toEqual({ name: 'pack' });
    expect(parseRoute('/schedule')).toEqual({ name: 'schedule' });
    expect(parseRoute('/calibration')).toEqual({ name: 'calibration' });
    expect(parseRoute('/jobs')).toEqual({ name: 'jobs' });
    expect(parseRoute('/admin')).toEqual({ name: 'admin' });
  });
  test('job detail keeps the share-link shape /jobs/:id', () => {
    expect(parseRoute('/jobs/7f3a9c-01')).toEqual({ name: 'job', id: '7f3a9c-01' });
    expect(parseRoute('/jobs/7f3a9c-01/')).toEqual({ name: 'job', id: '7f3a9c-01' });
  });
  test('a pack job opens the packing workspace /pack/:id', () => {
    expect(parseRoute('/pack/03bc69b1-a474')).toEqual({ name: 'pack', jobId: '03bc69b1-a474' });
    expect(parseRoute('/pack/Bad Id')).toEqual({ name: 'pack' });
  });
  test('login steps', () => {
    expect(parseRoute('/login')).toEqual({ name: 'login', step: 'signin' });
    expect(parseRoute('/login/register')).toEqual({ name: 'login', step: 'register' });
    expect(parseRoute('/login/forgot')).toEqual({ name: 'login', step: 'forgot' });
    expect(parseRoute('/login/sent')).toEqual({ name: 'login', step: 'sent' });
    expect(parseRoute('/login/code')).toEqual({ name: 'login', step: 'code' });
    expect(parseRoute('/login/other')).toEqual({ name: 'login', step: 'signin' });
  });
});

describe('routePath', () => {
  test('round-trips', () => {
    for (const path of ['/pack', '/pack/abc', '/schedule', '/calibration', '/jobs', '/jobs/abc', '/admin', '/login', '/login/code']) {
      expect(routePath(parseRoute(path))).toBe(path);
    }
  });
});
