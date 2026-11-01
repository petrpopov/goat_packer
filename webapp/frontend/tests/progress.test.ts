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
    const s = stripStage(stage({ id: 'final_slice', visited: ['probe_slice', 'packing', 'sample_slice', 'assembling', 'final_slice'] }));
    expect(s).toEqual({ label: 'Нарезка итоговой пластины', step: 5, steps: 6 });
  });
  test('re-slice after a repack is one more step with its own label', () => {
    const visited = ['probe_slice', 'packing', 'assembling', 'final_slice', 'gcode_check', 'repacking'];
    const s = stripStage(stage({ id: 'final_slice', visited, attempt: 2 }));
    expect(s).toEqual({ label: 'Повторная нарезка', step: 7, steps: 8 });
  });
  test('a finish job counts from its own first stage', () => {
    expect(stripStage(stage({ id: 'final_slice', visited: ['final_slice'] }))).toEqual({ label: 'Нарезка итоговой пластины', step: 1, steps: 2 });
  });
});

describe('stageItems', () => {
  test('done, current, then required pending; optional stages only once visited', () => {
    const items = stageItems(stage({ id: 'final_slice', visited: ['probe_slice', 'packing', 'assembling', 'final_slice'] }));
    expect(items.map((i) => [i.label, i.state])).toEqual([
      ['Пробная нарезка 1 объекта', 'done'], ['Раскладка', 'done'], ['Сборка проекта', 'done'],
      ['Нарезка итоговой пластины', 'current'], ['Запись результата', 'pending'],
    ]);
  });
});
