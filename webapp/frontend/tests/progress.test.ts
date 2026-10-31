import { describe, expect, test } from 'bun:test';
import type { JobStage } from '../src/api/types';
import {
  formatDuration,
  looksLikeJson,
  objectsText,
  packSteps,
  remainingText,
  stageElapsed,
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

describe('pack stepper', () => {
  test('current highlighted, earlier done, optional hidden until visited', () => {
    const steps = packSteps(stage({ id: 'final_slice', objects: 37, visited: ['probe_slice', 'packing', 'assembling', 'final_slice'] }));
    expect(steps.map((s) => s.id)).toEqual(['probe_slice', 'packing', 'assembling', 'final_slice', 'writing']);
    expect(steps.map((s) => s.state)).toEqual(['done', 'done', 'done', 'current', 'pending']);
    expect(steps[3].label).toBe('Нарезка итоговой пластины (37 объектов)');
  });

  test('repack and window stages appear once visited; reslice keeps repack done', () => {
    const visited = ['probe_slice', 'packing', 'assembling', 'final_slice', 'gcode_check', 'repacking'];
    const steps = packSteps(stage({ id: 'final_slice', visited, attempt: 2 }));
    expect(steps.map((s) => s.id)).toContain('repacking');
    expect(steps.find((s) => s.id === 'repacking')?.state).toBe('done');
    expect(steps.find((s) => s.id === 'gcode_check')?.state).toBe('done');
    expect(steps.find((s) => s.id === 'final_slice')?.state).toBe('current');
    expect(steps.map((s) => s.id)).not.toContain('window_slice');

    const window = packSteps(stage({ id: 'window_slice', visited: [...visited, 'window_slice'] }));
    expect(window.find((s) => s.id === 'window_slice')?.state).toBe('current');
  });

  test('skipped stage before current (grid layout) counts as done', () => {
    const steps = packSteps(stage({ id: 'assembling', visited: ['probe_slice', 'assembling'] }));
    expect(steps.find((s) => s.id === 'packing')?.state).toBe('done');
  });
});

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
    expect(remainingText(400, 317)).toBe('дольше обычного');
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
