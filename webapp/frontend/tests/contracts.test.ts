import { afterEach, describe, expect, test } from 'bun:test';
import { api, ApiError } from '../src/api/http';
import { detectProfile, deleteJob } from '../src/api/services';
import { buildNozzleClean, buildTimeFields, clockSeconds, numberInRange, validTimeWindow } from '../src/utils/forms';
import { formatHoursMinutes, packTimeLines } from '../src/utils/format';

const originalFetch = globalThis.fetch;
afterEach(() => {
  globalThis.fetch = originalFetch;
});

describe('form contracts', () => {
  test('global every=1 is explicit, not the CLI default every=10', () => {
    expect(buildNozzleClean('global', '', '1', '1')).toBe('global,every=1');
    expect(buildNozzleClean('global', '', '10', '1')).toBe('global,every=10');
    expect(buildNozzleClean('from-z', '6.6', '10', '2')).toBe('from-z=6.6,every=2');
    expect(buildNozzleClean('off', '', '', '')).toBe('');
  });
  test('no silent invalid numeric coercion', () => {
    for (const bad of ['', '0', '-1', '1.5', 'abc', 'Infinity']) {
      expect(() => buildNozzleClean('global', '', bad, '1')).toThrow();
    }
    for (const bad of ['', '0', '-1', 'abc', 'Infinity']) {
      expect(() => buildNozzleClean('from-z', bad, '1', '1')).toThrow();
    }
    expect(numberInRange('', 0, 600)).toBe(false);
    expect(numberInRange('601', 0, 600)).toBe(false);
    expect(numberInRange('10.5', 0, 600, true)).toBe(false);
    expect(numberInRange('0.05', 0.05, 10)).toBe(true);
  });
  test('frontend uses the same 0..47 clock and ordered-window rules as backend', () => {
    expect(clockSeconds('34:00')).toBe(122400);
    for (const bad of ['48:00', '99:00', '10:60', '', '-1:00']) expect(clockSeconds(bad)).toBeNull();
    expect(validTimeWindow('10:00', '23:00', '34:00')).toBe(true);
    expect(validTimeWindow('23:00', '10:00', '34:00')).toBe(false);
    expect(validTimeWindow('10:00', '10:00', '34:00')).toBe(false);
  });
});

describe('time window and AMS correction', () => {
  const base = { windowOn: false, windowHours: '12', marginPct: '10', amsOn: false, changeSeconds: '95' };
  test('nothing is sent unless the user enabled it', () => {
    expect(buildTimeFields(base, true)).toEqual([]);
  });
  test('window sends hours and margin; AMS sends the flag and seconds', () => {
    expect(buildTimeFields({ ...base, windowOn: true, amsOn: true, changeSeconds: '80' }, true)).toEqual([
      ['timeWindowHours', '12'],
      ['timeMarginPct', '10'],
      ['amsCorrection', 'true'],
      ['changeSeconds', '80'],
    ]);
    // Пустая длительность — значение профиля, отдельно не шлётся.
    expect(buildTimeFields({ ...base, amsOn: true, changeSeconds: ' ' }, true)).toEqual([['amsCorrection', 'true']]);
  });
  test('AMS correction is dropped for a printer without change time (tool changer)', () => {
    expect(buildTimeFields({ ...base, amsOn: true }, false)).toEqual([]);
  });
  test('out-of-range values are rejected like on the backend', () => {
    for (const windowHours of ['0.4', '241', '', 'abc']) {
      expect(() => buildTimeFields({ ...base, windowOn: true, windowHours }, true)).toThrow();
    }
    for (const marginPct of ['-1', '51']) {
      expect(() => buildTimeFields({ ...base, windowOn: true, marginPct }, true)).toThrow();
    }
    for (const changeSeconds of ['-1', '601']) {
      expect(() => buildTimeFields({ ...base, amsOn: true, changeSeconds }, true)).toThrow();
    }
  });
  test('result shows corrected time and, with a window, K of M', () => {
    expect(formatHoursMinutes(41580)).toBe('11 ч 33 мин');
    expect(formatHoursMinutes(3900)).toBe('1 ч 05 мин');
    const time = { windowHours: 12, marginPct: 10, chosenCount: 12, fullCount: 30, correctedSeconds: 41580 };
    expect(packTimeLines(time)).toEqual(['Оценка с коррекцией: 11 ч 33 мин', 'окно 12 ч, на столе 12 из 30']);
    expect(packTimeLines({ ...time, windowHours: undefined })).toEqual(['Оценка с коррекцией: 11 ч 33 мин']);
  });
});

describe('HTTP contracts', () => {
  test('multipart passes file and AbortSignal, without overriding boundary', async () => {
    const controller = new AbortController();
    globalThis.fetch = (async (url, init) => {
      expect(url).toBe('/api/profiles/detect');
      expect(init?.body).toBeInstanceOf(FormData);
      expect(new Headers(init?.headers).has('content-type')).toBe(false);
      expect(init?.signal).toBe(controller.signal);
      return Response.json({ profileId: 'bambu_p1s', printerModel: 'Bambu Lab P1S' });
    }) as typeof fetch;
    expect((await detectProfile(new File(['zip'], 'model.3mf'), controller.signal)).profileId).toBe('bambu_p1s');
  });
  test('old omitted nullable fields cannot become an undefined profile selection', async () => {
    globalThis.fetch = (async () => Response.json({})) as typeof fetch;
    expect(await detectProfile(new File(['zip'], 'model.3mf'))).toEqual({ profileId: null, printerModel: null });
  });
  test('server errors remain errors instead of missing-printer results', async () => {
    globalThis.fetch = (async () =>
      Response.json({ code: 'BAD_REQUEST', message: 'Повреждён ZIP' }, { status: 400 })) as typeof fetch;
    try {
      await detectProfile(new File(['bad'], 'model.3mf'));
      throw new Error('expected rejection');
    } catch (error) {
      expect(error).toBeInstanceOf(ApiError);
      expect((error as ApiError).status).toBe(400);
      expect((error as ApiError).message).toBe('Повреждён ZIP');
    }
  });
  test('DELETE encodes IDs and accepts an empty 204 response', async () => {
    globalThis.fetch = (async (url, init) => {
      expect(url).toBe('/api/jobs/a%2Fb');
      expect(init?.method).toBe('DELETE');
      return new Response(null, { status: 204 });
    }) as typeof fetch;
    await deleteJob('a/b');
  });
  test('non-JSON proxy failures have a useful HTTP error', async () => {
    globalThis.fetch = (async () => new Response('proxy error', { status: 502 })) as typeof fetch;
    expect(api.get('/profiles')).rejects.toThrow('HTTP 502');
  });
});
