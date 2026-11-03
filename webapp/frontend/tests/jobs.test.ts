import { describe, expect, test } from 'bun:test';
import type { PackJobInfo } from '../src/api/types';
import { TOWER_GUI_WARNING, towerWarning } from '../src/utils/jobs';

const packed = (tower: PackJobInfo['tower']): PackJobInfo => ({
  objectCount: 30,
  profileId: 'test_wide_bed',
  clearanceMm: 1,
  tower,
});

describe('prime tower warning on the pack result', () => {
  const tower = { anchorXMm: 10, anchorYMm: 10, widthMm: 20, depthMm: 20 };

  test('a finished plate with a tower asks to drag the tower back after opening in the GUI', () => {
    expect(towerWarning('SUCCEEDED', packed(tower))).toBe(TOWER_GUI_WARNING);
    expect(TOWER_GUI_WARNING).toContain('Bambu Studio');
    expect(TOWER_GUI_WARNING).toContain('Snapmaker Orca');
  });

  test('no tower, no result or an unfinished job shows nothing', () => {
    expect(towerWarning('SUCCEEDED', packed(null))).toBeNull();
    expect(towerWarning('SUCCEEDED', packed(undefined))).toBeNull();
    expect(towerWarning('SUCCEEDED', null)).toBeNull();
    expect(towerWarning('RUNNING', packed(tower))).toBeNull();
    expect(towerWarning('FAILED', packed(tower))).toBeNull();
    expect(towerWarning(undefined, packed(tower))).toBeNull();
  });
});
