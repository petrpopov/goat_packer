import { describe, expect, test } from 'bun:test';
import {
  customPrinterProfile,
  effectiveSlicer,
  filterPrinters,
  foundPrintersMessage,
  groupPrinters,
  slicerLabel,
  slicerOptions,
  vendorOf,
} from '../src/utils/printers';

const p = (id: string, name: string, vendor: string) => ({ id, name, vendor });
const list = [p('b', 'Gamma Works GW4', 'Gamma Works'), p('a', 'Alpha Labs A100', 'Alpha Labs'), p('c', 'Gamma Works CORE', 'Gamma Works'), p('d', 'Zed-3 V3 SE', 'Beta Forge')];

describe('printers', () => {
  test('vendors A→Z, models A→Z inside', () => {
    expect(groupPrinters(list).map((g) => [g.vendor, g.items.map((i) => i.name)])).toEqual([
      ['Alpha Labs', ['Alpha Labs A100']],
      ['Beta Forge', ['Zed-3 V3 SE']],
      ['Gamma Works', ['Gamma Works CORE', 'Gamma Works GW4']],
    ]);
  });
  test('search matches vendor or model, case/space-insensitive, keeps grouping', () => {
    expect(groupPrinters(filterPrinters(list, 'gamma works gw')).flatMap((g) => g.items.map((i) => i.id))).toEqual(['b']);
    expect(groupPrinters(filterPrinters(list, 'beta f')).map((g) => g.vendor)).toEqual(['Beta Forge']);
    expect(filterPrinters(list, '').length).toBe(4);
  });
  test('vendor falls back to the name minus its last word until the API sends it', () => {
    expect(vendorOf({ name: 'Alpha Labs A100' })).toBe('Alpha Labs');
    expect(vendorOf({ name: 'Delta D1', vendor: 'Delta Tools' })).toBe('Delta Tools');
    expect(vendorOf({ name: 'Solo' })).toBe('Solo');
    expect(groupPrinters([{ id: 'y', name: 'Alpha Labs A200' }, { id: 'x', name: 'Alpha Labs A100' }]).map((g) => [g.vendor, g.items.map((i) => i.id)])).toEqual([
      ['Alpha Labs', ['x', 'y']],
    ]);
  });
  test('found-printers announcement uses Russian plural forms', () => {
    expect([0, 1, 2, 5, 11, 21, 22, 112, 148].map(foundPrintersMessage)).toEqual([
      'Найдено 0 принтеров',
      'Найдено 1 принтер',
      'Найдено 2 принтера',
      'Найдено 5 принтеров',
      'Найдено 11 принтеров',
      'Найдено 21 принтер',
      'Найдено 22 принтера',
      'Найдено 112 принтеров',
      'Найдено 148 принтеров',
    ]);
  });
  test('a 3MF project bed is always the bed: origin and exclusions kept, only the margin is ours', () => {
    const projectBed = { printableArea: [-5, 0, 245, 210], exclusionRectangles: [[0, 0, 20, 30]], printableHeight: 220 };
    const profile = customPrinterProfile({ width: 250, depth: 210, height: 220, margin: 7 }, projectBed);
    expect(profile.printableArea).toEqual([-5, 0, 245, 210]);
    expect(profile.exclusionRectangles).toEqual([[0, 0, 20, 30]]);
    expect(profile.edgeMarginMm).toBe(7);
    expect(customPrinterProfile({ width: 300, depth: 200, height: 220, margin: 5 }).printableArea).toEqual([0, 0, 300, 200]);
  });
});

describe('slicer choice for STL', () => {
  const bambu = {
    slicerKind: 'bambu',
    slicers: [
      { kind: 'bambu', gui: 'bambu-02.08' },
      { kind: 'orca', gui: 'orca-2.4' },
    ],
  };
  test('options come from the profile, default first; an old answer offers its own kind', () => {
    expect(slicerOptions(bambu).map((o) => o.kind)).toEqual(['bambu', 'orca']);
    expect(slicerOptions({ slicerKind: 'orca' })).toEqual([{ kind: 'orca', gui: '' }]);
  });
  test('labels follow the GUI: Snapmaker printers open in Snapmaker Orca', () => {
    expect(slicerLabel({ kind: 'bambu', gui: 'bambu-02.08' })).toBe('Bambu Studio');
    expect(slicerLabel({ kind: 'orca', gui: 'orca-2.4' })).toBe('OrcaSlicer');
    expect(slicerLabel({ kind: 'orca', gui: 'snapmaker-orca-2.4' })).toBe('Snapmaker Orca');
    expect(slicerLabel({ kind: 'bambu', gui: '' })).toBe('Bambu Studio');
  });
  test('a choice the printer does not offer falls back to its default', () => {
    expect(effectiveSlicer(bambu.slicers, 'orca')).toBe('orca');
    expect(effectiveSlicer(bambu.slicers, '')).toBe('bambu');
    expect(effectiveSlicer([{ kind: 'orca', gui: 'snapmaker-orca-2.4' }], 'bambu')).toBe('orca');
  });
});
