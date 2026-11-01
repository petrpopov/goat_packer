import { describe, expect, test } from 'bun:test';
import type { JobStage } from '../src/api/types';
import {
  formatDuration,
  looksLikeJson,
  objectsText,
  remainingText,
  sceneClip,
  stageElapsed,
  stageSegments,
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

describe('stageSegments', () => {
  test('weights follow the backend bands; current stage is filled by percent', () => {
    const segs = stageSegments(stage({ id: 'final_slice', visited: ['probe_slice', 'packing', 'assembling', 'final_slice'] }), 55.5);
    expect(segs.map((s) => [s.id, s.weight, s.state])).toEqual([
      ['probe_slice', 14, 'done'], ['packing', 15, 'done'], ['assembling', 3, 'done'], ['final_slice', 45, 'current'],
      ['gcode_check', 4, 'pending'], ['repacking', 4, 'pending'], ['window_slice', 9, 'pending'], ['writing', 4, 'pending'],
    ]);
    expect(segs[3].fill).toBeCloseTo(0.5);
  });
  test('optional stages passed without a visit are skipped', () => {
    const segs = stageSegments(stage({ id: 'writing', visited: ['probe_slice', 'packing', 'assembling', 'final_slice', 'gcode_check'] }), 97);
    expect(segs.find((s) => s.id === 'repacking')!.state).toBe('skipped');
    expect(segs.find((s) => s.id === 'gcode_check')!.state).toBe('done');
  });
  test('re-slice after a repack is its own current segment after the repack', () => {
    const visited = ['probe_slice', 'packing', 'assembling', 'final_slice', 'gcode_check', 'repacking'];
    const segs = stageSegments(stage({ id: 'final_slice', visited, attempt: 2 }), 88);
    expect(segs.map((s) => [s.key, s.state])).toEqual([
      ['probe_slice', 'done'], ['packing', 'done'], ['assembling', 'done'], ['final_slice', 'done'],
      ['gcode_check', 'done'], ['repacking', 'done'], ['final_slice:repeat', 'current'],
      ['window_slice', 'pending'], ['writing', 'pending'],
    ]);
    const repeat = segs.find((s) => s.state === 'current')!;
    expect(repeat.label).toBe('Повторная нарезка');
    // Полоса повтора: от конца перепаковки (86) до середины пути к 99 (92.5).
    expect(repeat.weight).toBeCloseTo(6.5);
    expect(repeat.fill).toBeCloseTo(2 / 6.5);
  });
  test('the check after the re-slice repeats too, the window stays ahead', () => {
    const visited = ['probe_slice', 'packing', 'assembling', 'final_slice', 'gcode_check', 'repacking'];
    const segs = stageSegments(stage({ id: 'gcode_check', visited, attempt: 2 }), 90);
    expect(segs.filter((s) => s.state === 'current').map((s) => [s.key, s.label])).toEqual([
      ['gcode_check:repeat', 'Повторная проверка поддержек'],
    ]);
    expect(segs.find((s) => s.key === 'gcode_check')!.state).toBe('done');
  });
  test('a second repack is not a repeat: it is already the furthest stage', () => {
    const visited = ['probe_slice', 'packing', 'assembling', 'final_slice', 'gcode_check', 'repacking'];
    const segs = stageSegments(stage({ id: 'repacking', visited, attempt: 2 }), 90);
    expect(segs.map((s) => s.key)).not.toContain('repacking:repeat');
    expect(segs.find((s) => s.id === 'repacking')!.state).toBe('current');
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
