import { describe, expect, test } from 'bun:test';
import type { PackJobInfo } from '../src/api/types';
import { SNAPMAKER_TOWER_CONFLICT_NOTE, TOWER_GUI_WARNING, snapmakerTowerNote, towerWarning } from '../src/utils/jobs';

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

describe('Snapmaker tower conflict note on the pack result', () => {
  const tower = { anchorXMm: 10, anchorYMm: 10, widthMm: 20, depthMm: 20 };
  const snapmaker = { name: 'Snapmaker U1', vendor: 'Snapmaker' };

  test('a finished Snapmaker plate with a tower says how to resolve the first-layer conflict', () => {
    expect(snapmakerTowerNote('SUCCEEDED', packed(tower), snapmaker)).toBe(SNAPMAKER_TOWER_CONFLICT_NOTE);
    expect(SNAPMAKER_TOWER_CONFLICT_NOTE).toContain('Snapmaker Orca');
    expect(SNAPMAKER_TOWER_CONFLICT_NOTE).toContain('1–2 мм');
  });

  test('the vendor is also taken from the name when the profile has none', () => {
    expect(snapmakerTowerNote('SUCCEEDED', packed(tower), { name: 'Snapmaker J1' })).toBe(SNAPMAKER_TOWER_CONFLICT_NOTE);
  });

  test('other vendors, no tower or an unfinished job show nothing', () => {
    expect(snapmakerTowerNote('SUCCEEDED', packed(tower), { name: 'Bambu Lab P1S', vendor: 'Bambu Lab' })).toBeNull();
    expect(snapmakerTowerNote('SUCCEEDED', packed(null), snapmaker)).toBeNull();
    expect(snapmakerTowerNote('RUNNING', packed(tower), snapmaker)).toBeNull();
    expect(snapmakerTowerNote('SUCCEEDED', packed(tower), null)).toBeNull();
  });
});
