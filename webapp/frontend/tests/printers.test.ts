import { describe, expect, test } from 'bun:test';
import { filterPrinters, groupPrinters, vendorOf } from '../src/utils/printers';

const p = (id: string, name: string, vendor: string) => ({ id, name, vendor });
const list = [p('b', 'Prusa MK4S', 'Prusa'), p('a', 'Bambu Lab P1S', 'Bambu Lab'), p('c', 'Prusa CORE One', 'Prusa'), p('d', 'Ender-3 V3 SE', 'Creality')];

describe('printers', () => {
  test('vendors A→Z, models A→Z inside', () => {
    expect(groupPrinters(list).map((g) => [g.vendor, g.items.map((i) => i.name)])).toEqual([
      ['Bambu Lab', ['Bambu Lab P1S']],
      ['Creality', ['Ender-3 V3 SE']],
      ['Prusa', ['Prusa CORE One', 'Prusa MK4S']],
    ]);
  });
  test('search matches vendor or model, case/space-insensitive, keeps grouping', () => {
    expect(groupPrinters(filterPrinters(list, 'prusa mk')).flatMap((g) => g.items.map((i) => i.id))).toEqual(['b']);
    expect(groupPrinters(filterPrinters(list, 'creal')).map((g) => g.vendor)).toEqual(['Creality']);
    expect(filterPrinters(list, '').length).toBe(4);
  });
  test('vendor falls back to the name minus its last word until the API sends it', () => {
    expect(vendorOf({ name: 'Bambu Lab P1S' })).toBe('Bambu Lab');
    expect(vendorOf({ name: 'Snapmaker U1', vendor: 'Snapmaker' })).toBe('Snapmaker');
    expect(vendorOf({ name: 'Voron' })).toBe('Voron');
    expect(groupPrinters([{ id: 'y', name: 'Bambu Lab P1S' }, { id: 'x', name: 'Bambu Lab A2L' }]).map((g) => [g.vendor, g.items.map((i) => i.id)])).toEqual([
      ['Bambu Lab', ['x', 'y']],
    ]);
  });
});
