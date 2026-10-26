import { describe, expect, test } from 'bun:test';
import {
  CALIBRATION_VALUES_KEY,
  CHANGE_SECONDS_KEY,
  SUSPICIOUS_TIME_WARNING,
  calibrationValues,
  computeChangeSeconds,
  defaultChangeSeconds,
  parseCalibrationJson,
  parseClockMinutes,
  printDurationSeconds,
  readStoredCalibration,
  readStoredChangeSeconds,
  storeCalibration,
  storeChangeSeconds,
} from '../src/utils/calibration';
import type { KeyValueStorage } from '../src/utils/calibration';
import { buildSupportOverlap } from '../src/utils/forms';
import { parseInline, parseMarkdown } from '../src/utils/markdown';

class MemoryStorage implements KeyValueStorage {
  readonly data = new Map<string, string>();
  getItem(key: string) {
    return this.data.get(key) ?? null;
  }
  setItem(key: string, value: string) {
    this.data.set(key, value);
  }
}

// calibration.json фейкового прогона: A 25 мин, B 40 мин по оценке, 10 смен по 20 с в слайсере.
const values = { estimateASeconds: 1500, estimateBSeconds: 2400, changes: 10, slicerChangeSeconds: 20 };

describe('AMS calibration calculator', () => {
  test('clock parsing: HH:MM within a day only', () => {
    expect(parseClockMinutes('9:05')).toBe(545);
    expect(parseClockMinutes(' 23:59 ')).toBe(1439);
    for (const bad of ['24:00', '12:60', '', '12', 'ab:cd', '-1:00']) expect(parseClockMinutes(bad)).toBeNull();
  });

  test('finish earlier than start means after midnight (+24 h)', () => {
    expect(printDurationSeconds('10:00', '10:30')).toBe(1800);
    expect(printDurationSeconds('23:50', '00:20')).toBe(1800);
    expect(printDurationSeconds('10:00', '10:00')).toBe(0);
    expect(printDurationSeconds('10:00', 'x')).toBeNull();
  });

  test('extra = ((factB − estB) − (factA − estA)) / changes; real = extra + slicer, rounded to 1 s', () => {
    // A: 30 мин факт (оценка 25) → +300 с; B: 1 ч 00 мин факт (оценка 40) → +1200 с; (1200−300)/10 = 90.
    const result = computeChangeSeconds(
      { aStart: '10:00', aFinish: '10:30', bStart: '11:00', bFinish: '12:00' },
      values
    );
    expect(result).toEqual({ ok: true, extraSeconds: 90, realSeconds: 110, warning: null });

    // Дробный результат округляется: (1200−300)/7 = 128.57 → 128.57 + 20.4 = 148.97 → 149.
    const fractional = computeChangeSeconds(
      { aStart: '10:00', aFinish: '10:30', bStart: '11:00', bFinish: '12:00' },
      { ...values, changes: 7, slicerChangeSeconds: 20.4 }
    );
    expect(fractional.ok && fractional.realSeconds).toBe(149);
  });

  test('B over midnight is handled like any other print', () => {
    const result = computeChangeSeconds(
      { aStart: '22:00', aFinish: '22:30', bStart: '23:30', bFinish: '00:30' },
      values
    );
    expect(result.ok && result.realSeconds).toBe(110);
  });

  test('extra outside 0..200 s warns but still shows the value', () => {
    // B быстрее оценки: отрицательное «лишнее».
    const negative = computeChangeSeconds(
      { aStart: '10:00', aFinish: '10:30', bStart: '11:00', bFinish: '11:35' },
      values
    );
    expect(negative).toEqual({ ok: true, extraSeconds: -60, realSeconds: -40, warning: SUSPICIOUS_TIME_WARNING });
    // 300 с на смену — слишком много.
    const huge = computeChangeSeconds({ aStart: '10:00', aFinish: '10:25', bStart: '11:00', bFinish: '12:30' }, values);
    expect(huge.ok && huge.warning).toBe(SUSPICIOUS_TIME_WARNING);
    expect(huge.ok && huge.realSeconds).toBe(320);
    // Граница 200 с включительно — без предупреждения.
    const edge = computeChangeSeconds(
      { aStart: '10:00', aFinish: '10:25', bStart: '11:00', bFinish: '12:10' },
      { ...values, estimateBSeconds: 2200 }
    );
    expect(edge).toEqual({ ok: true, extraSeconds: 200, realSeconds: 220, warning: null });
  });

  test('missing or invalid inputs are errors, not NaN', () => {
    const clocks = { aStart: '10:00', aFinish: '10:30', bStart: '11:00', bFinish: '' };
    expect(computeChangeSeconds(clocks, values).ok).toBe(false);
    const full = { ...clocks, bFinish: '12:00' };
    expect(computeChangeSeconds(full, { ...values, changes: 0 }).ok).toBe(false);
    expect(computeChangeSeconds(full, { ...values, changes: 2.5 }).ok).toBe(false);
    expect(computeChangeSeconds(full, { ...values, estimateASeconds: NaN }).ok).toBe(false);
  });

  test('values can be pasted from calibration.json or taken from the job', () => {
    const text = JSON.stringify({
      variant: 'normal',
      changes_expected: 10,
      changes_sliced: 10,
      estimate_a_s: 1500,
      estimate_b_s: 2400,
      slicer_change_s: 20,
    });
    expect(parseCalibrationJson(text)).toEqual(values);
    expect(parseCalibrationJson('{"estimate_a_s": 1}')).toBeNull();
    expect(parseCalibrationJson('not json')).toBeNull();
    expect(
      calibrationValues({ estimateASeconds: 1500, estimateBSeconds: 2400, changesSliced: 10, slicerChangeSeconds: 20 })
    ).toEqual(values);
  });
});

describe('stored correction', () => {
  test('per-profile change seconds survive a reload and ignore garbage', () => {
    const storage = new MemoryStorage();
    expect(readStoredChangeSeconds(storage)).toEqual({});
    storeChangeSeconds('bambu_p1s', 104, storage);
    storeChangeSeconds('bambu_a2l', 88, storage);
    storeChangeSeconds('bambu_p1s', 110, storage);
    expect(readStoredChangeSeconds(storage)).toEqual({ bambu_p1s: 110, bambu_a2l: 88 });
    storage.setItem(CHANGE_SECONDS_KEY, JSON.stringify({ a: 'x', b: -5, c: 700, d: 50 }));
    expect(readStoredChangeSeconds(storage)).toEqual({ d: 50 });
    storage.setItem(CHANGE_SECONDS_KEY, '{broken');
    expect(readStoredChangeSeconds(storage)).toEqual({});
    expect(readStoredChangeSeconds(null)).toEqual({});
  });

  test('last calibration values are kept per profile for a later visit', () => {
    const storage = new MemoryStorage();
    expect(readStoredCalibration('bambu_p1s', storage)).toBeNull();
    storeCalibration('bambu_p1s', values, storage);
    expect(readStoredCalibration('bambu_p1s', storage)).toEqual(values);
    expect(readStoredCalibration('bambu_a2l', storage)).toBeNull();
    storage.setItem(CALIBRATION_VALUES_KEY, JSON.stringify({ bambu_p1s: { estimateASeconds: 'x' } }));
    expect(readStoredCalibration('bambu_p1s', storage)).toBeNull();
  });

  test('AMS seconds field: stored calibration first, then the profile value; tool changer stays empty', () => {
    expect(defaultChangeSeconds(91, 110)).toBe('110');
    expect(defaultChangeSeconds(91, undefined)).toBe('91');
    expect(defaultChangeSeconds(null, 110)).toBe('');
  });
});

describe('support overlap field', () => {
  test('empty means unlimited and sends nothing; 0..20 mm is sent as a number', () => {
    expect(buildSupportOverlap('')).toEqual([]);
    expect(buildSupportOverlap('  ')).toEqual([]);
    expect(buildSupportOverlap('0')).toEqual([['supportOverlapMm', '0']]);
    expect(buildSupportOverlap('2.50')).toEqual([['supportOverlapMm', '2.5']]);
    expect(buildSupportOverlap('20')).toEqual([['supportOverlapMm', '20']]);
    for (const bad of ['-0.1', '20.1', 'abc', 'Infinity']) expect(() => buildSupportOverlap(bad)).toThrow();
  });
});

describe('instructions markdown', () => {
  test('inline code and bold become typed parts, never HTML', () => {
    expect(parseInline('файл `calib_A.3mf` и **обычный** <b>x</b>')).toEqual([
      { kind: 'text', text: 'файл ' },
      { kind: 'code', text: 'calib_A.3mf' },
      { kind: 'text', text: ' и ' },
      { kind: 'bold', text: 'обычный' },
      { kind: 'text', text: ' <b>x</b>' },
    ]);
  });

  test('headings, wrapped list items, numbered lists and code fences', () => {
    const blocks = parseMarkdown(
      '# Заголовок\n\nАбзац\nпродолжение.\n\n- `calib_A.3mf` — кубик\n  без смен.\n- второй\n\n1. Раз\n2. Два\n\n```\nлишнее = (B − A) / 10\n```\n## Итог'
    );
    expect(blocks.map((block) => block.kind)).toEqual(['heading', 'paragraph', 'list', 'list', 'code', 'heading']);
    expect(blocks[1]).toEqual({ kind: 'paragraph', inline: [{ kind: 'text', text: 'Абзац продолжение.' }] });
    const bullets = blocks[2];
    expect(bullets.kind === 'list' && !bullets.ordered && bullets.items.length).toBe(2);
    expect(bullets.kind === 'list' && bullets.items[0][1]).toEqual({ kind: 'text', text: ' — кубик без смен.' });
    expect(blocks[3].kind === 'list' && blocks[3].ordered).toBe(true);
    expect(blocks[4]).toEqual({ kind: 'code', text: 'лишнее = (B − A) / 10' });
  });
});

describe('calibration job HTTP contract', () => {
  test('local mode posts the multipart form to /api/calibration/jobs', async () => {
    const { startCalibrationJob } = await import('../src/api/services');
    const originalFetch = globalThis.fetch;
    try {
      globalThis.fetch = (async (url, init) => {
        expect(url).toBe('/api/calibration/jobs');
        expect(init?.method).toBe('POST');
        const body = init?.body as FormData;
        expect(body.get('variant')).toBe('perfectionist');
        expect(body.get('profileId')).toBe('bambu_p1s');
        return Response.json({ jobId: 'job-1' }, { status: 202 });
      }) as typeof fetch;
      const form = new FormData();
      form.append('profileId', 'bambu_p1s');
      form.append('variant', 'perfectionist');
      expect((await startCalibrationJob(form)).jobId).toBe('job-1');
    } finally {
      globalThis.fetch = originalFetch;
    }
  });
});
