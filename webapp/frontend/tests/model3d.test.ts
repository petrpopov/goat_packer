import { describe, expect, test } from 'bun:test';
import { strToU8, zipSync } from 'fflate';
import { parseModel } from '../src/scene/model3d';

/** Binary STL from triangles (each 9 numbers). */
function binaryStl(tris: number[][]): ArrayBuffer {
  const buf = new ArrayBuffer(84 + tris.length * 50);
  const dv = new DataView(buf);
  dv.setUint32(80, tris.length, true);
  tris.forEach((t, i) => t.forEach((v, k) => dv.setFloat32(84 + i * 50 + 12 + k * 4, v, true)));
  return buf;
}
// Тетраэдр 10×20×5 мм со смещением — проверяем центрирование.
const TET = [
  [100, 100, 7, 110, 100, 7, 100, 120, 7],
  [100, 100, 7, 100, 120, 7, 100, 100, 12],
  [100, 100, 7, 100, 100, 12, 110, 100, 7],
  [110, 100, 7, 100, 100, 12, 100, 120, 7],
];

describe('parseModel', () => {
  test('binary STL is centred: XY bbox centre at 0, min z at 0', () => {
    const m = parseModel('part.stl', binaryStl(TET));
    expect(m.triangles).toBe(4);
    expect(m.size.map((v) => Math.round(v * 1000) / 1000)).toEqual([10, 20, 5]);
    let minX = Infinity, maxX = -Infinity, minZ = Infinity;
    for (let i = 0; i < m.positions.length; i += 3) {
      minX = Math.min(minX, m.positions[i]); maxX = Math.max(maxX, m.positions[i]); minZ = Math.min(minZ, m.positions[i + 2]);
    }
    expect(minX).toBeCloseTo(-5); expect(maxX).toBeCloseTo(5); expect(minZ).toBeCloseTo(0);
  });

  test('ASCII STL', () => {
    const text = `solid t\n${TET.map((t) => `facet normal 0 0 0\nouter loop\n${[0, 3, 6].map((o) => `vertex ${t[o]} ${t[o + 1]} ${t[o + 2]}`).join('\n')}\nendloop\nendfacet`).join('\n')}\nendsolid t\n`;
    const m = parseModel('part.stl', strToU8(text).buffer as ArrayBuffer);
    expect(m.triangles).toBe(4);
    expect(m.size[1]).toBeCloseTo(20);
  });

  test('3MF follows build item → component (p:path) → mesh with transforms', () => {
    const mesh = `<model xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02"><resources><object id="1" type="model"><mesh><vertices>
      <vertex x="0" y="0" z="0"/><vertex x="4" y="0" z="0"/><vertex x="0" y="6" z="0"/><vertex x="0" y="0" z="3"/>
      </vertices><triangles><triangle v1="0" v2="1" v3="2"/><triangle v1="0" v2="2" v3="3"/><triangle v1="0" v2="3" v3="1"/><triangle v1="1" v2="3" v3="2"/></triangles></mesh></object></resources></model>`;
    // Компонент поворачивает на 90° вокруг Z: размеры X/Y меняются местами.
    const root = `<model xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02" xmlns:p="http://schemas.microsoft.com/3dmanufacturing/production/2015/06"><resources>
      <object id="2" type="model"><components><component objectid="1" p:path="/3D/Objects/mesh.model" transform="0 1 0 -1 0 0 0 0 1 10 20 -3"/></components></object>
      </resources><build><item objectid="2" transform="1 0 0 0 1 0 0 0 1 128 128 3"/></build></model>`;
    const zip = zipSync({ '3D/3dmodel.model': strToU8(root), '3D/Objects/mesh.model': strToU8(mesh) });
    const m = parseModel('project.3mf', zip.buffer as ArrayBuffer);
    expect(m.triangles).toBe(4);
    expect(m.size.map((v) => Math.round(v))).toEqual([6, 4, 3]);
  });

  test('unsupported extension throws a Russian message', () => {
    expect(() => parseModel('model.obj', new ArrayBuffer(10))).toThrow('Поддерживаются .3mf и .stl');
  });
});
