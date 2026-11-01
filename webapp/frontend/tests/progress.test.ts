import { describe, expect, test } from 'bun:test';
import type { JobStage } from '../src/api/types';
import {
  formatDuration,
  looksLikeJson,
  objectsText,
  remainingText,
  sceneClip,
  stageElapsed,
  stageItems,
  stageRows,
  stripStage,
  stageTimeLine,
} from '../src/utils/progress';

function stage(partial: Partial<JobStage>): JobStage {
  return {
    id: 'probe_slice',
    startedAtMs: 0,
    elapsedSeconds: 0,
    expectedSeconds: null,
    objects: null,
    attempt: 1,
    visited: [],
    detail: null,
    ...partial,
  };
}

describe('stage timing text', () => {
  test('durations', () => {
    expect(formatDuration(45.7)).toBe('45 с');
    expect(formatDuration(125)).toBe('2 мин 05 с');
    expect(formatDuration(3780)).toBe('1 ч 03 мин');
  });

  test('remaining only with packer estimate', () => {
    expect(remainingText(10, null)).toBeNull();
    expect(remainingText(100, 317)).toBe('осталось ≈ 4 мин');
    expect(remainingText(300, 317)).toBe('осталось < 1 мин');
    // До полутора оценок — ещё норма, не тревога.
    expect(remainingText(400, 317)).toBe('осталось < 1 мин');
    expect(remainingText(500, 317)).toBe('дольше обычного');
    expect(stageTimeLine(125, null)).toBe('идёт 2 мин 05 с');
    expect(stageTimeLine(20, 200)).toBe('идёт 20 с · осталось ≈ 3 мин');
  });

  test('elapsed ticks locally from the moment of receipt', () => {
    const s = stage({ elapsedSeconds: 30 });
    expect(stageElapsed(s, 1_000, 3_500)).toBe(32.5);
    expect(stageElapsed(s, 5_000, 3_000)).toBe(30);
    expect(stageElapsed(s, undefined, 9_999)).toBe(30);
  });

  test('plural objects', () => {
    expect(objectsText(1)).toBe('1 объект');
    expect(objectsText(3)).toBe('3 объекта');
    expect(objectsText(11)).toBe('11 объектов');
    expect(objectsText(37)).toBe('37 объектов');
    expect(objectsText(22)).toBe('22 объекта');
  });

  test('json is never shown, scheduler lines are', () => {
    expect(looksLikeJson('{"event":"slicer_output","stream":"stdout","message":"x"}')).toBe(true);
    expect(looksLikeJson('[Bambu Lab A2L] slicing sample 1/5: batch 8/30')).toBe(false);
    expect(looksLikeJson(null)).toBe(false);
  });
});

describe('sceneClip', () => {
  test('probe and final slice raise the cut plane through the stage band', () => {
    expect(sceneClip('probe_slice', 8)).toBeCloseTo(0.5);
    expect(sceneClip('final_slice', 33)).toBeCloseTo(0);
    expect(sceneClip('final_slice', 78)).toBeCloseTo(1);
  });
  test('other stages show whole objects', () => {
    expect(sceneClip('packing', 20)).toBe(1);
    expect(sceneClip('writing', 97)).toBe(1);
  });
});

describe('stripStage', () => {
  test('first pass: step = visited, steps = visited + required ahead', () => {
    const s = stripStage(
      stage({ id: 'final_slice', visited: ['probe_slice', 'packing', 'sample_slice', 'assembling', 'final_slice'] })
    );
    expect(s).toEqual({ label: 'Нарезка итоговой пластины', step: 5, steps: 6 });
  });
  test('re-slice after a repack is one more step with its own label', () => {
    const visited = ['probe_slice', 'packing', 'assembling', 'final_slice', 'gcode_check', 'repacking'];
    const s = stripStage(stage({ id: 'final_slice', visited, attempt: 2 }));
    expect(s).toEqual({ label: 'Повторная нарезка', step: 7, steps: 8 });
  });
  test('a finish job counts from its own first stage', () => {
    expect(stripStage(stage({ id: 'final_slice', visited: ['final_slice'] }))).toEqual({
      label: 'Нарезка итоговой пластины',
      step: 1,
      steps: 2,
    });
  });
});

describe('stageItems', () => {
  test('done, current, then required pending; optional stages only once visited', () => {
    const items = stageItems(
      stage({ id: 'final_slice', visited: ['probe_slice', 'packing', 'assembling', 'final_slice'] })
    );
    expect(items.map((i) => [i.label, i.state])).toEqual([
      ['Пробная нарезка 1 объекта', 'done'],
      ['Раскладка', 'done'],
      ['Сборка проекта', 'done'],
      ['Нарезка итоговой пластины', 'current'],
      ['Запись результата', 'pending'],
    ]);
  });
});

describe('stageRows', () => {
  const history = [
    { id: 'probe_slice', attempt: 1, startedAtMs: 0, endedAtMs: 41_000 },
    { id: 'packing', attempt: 1, startedAtMs: 41_000, endedAtMs: 50_000 },
    { id: 'assembling', attempt: 1, startedAtMs: 50_000, endedAtMs: null },
  ];

  test('finished job: every span timed, the open one closed by finishedAt', () => {
    const rows = stageRows(stage({ id: 'assembling', visited: ['probe_slice', 'packing', 'assembling'], history }), {
      running: false,
      failed: false,
      elapsedNow: 0,
      finishedAtMs: 52_000,
    });
    expect(rows.map((r) => [r.label, r.state, r.seconds])).toEqual([
      ['Пробная нарезка 1 объекта', 'done', 41],
      ['Раскладка', 'done', 9],
      ['Сборка проекта', 'done', 2],
    ]);
  });

  test('running job: current span counts live, required stages follow as pending', () => {
    const rows = stageRows(stage({ id: 'assembling', visited: ['probe_slice', 'packing', 'assembling'], history }), {
      running: true,
      failed: false,
      elapsedNow: 3,
      finishedAtMs: null,
    });
    expect(rows[2]).toMatchObject({ state: 'current', seconds: 3 });
    expect(rows.slice(3).map((r) => r.state)).toEqual(['pending', 'pending']);
  });

  test('failed job marks its last span; a repeat span gets the repeat label', () => {
    const repeat = [
      ...history.slice(0, 2),
      { id: 'final_slice', attempt: 1, startedAtMs: 50_000, endedAtMs: 60_000 },
      { id: 'final_slice', attempt: 2, startedAtMs: 60_000, endedAtMs: null },
    ];
    const rows = stageRows(
      stage({ id: 'final_slice', attempt: 2, visited: ['probe_slice', 'packing', 'final_slice'], history: repeat }),
      {
        running: false,
        failed: true,
        elapsedNow: 0,
        finishedAtMs: 65_000,
      }
    );
    expect(rows.at(-1)).toMatchObject({ label: 'Повторная нарезка', state: 'failed', seconds: 5 });
  });

  test('old jobs without history fall back to the plain list', () => {
    const rows = stageRows(stage({ id: 'packing', visited: ['probe_slice', 'packing'] }), {
      running: true,
      failed: false,
      elapsedNow: 1,
      finishedAtMs: null,
    });
    expect(rows.every((r) => r.seconds === null)).toBe(true);
  });
});
