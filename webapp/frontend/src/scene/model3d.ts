import type { ProjectionMask } from './projection';
import { strFromU8, unzipSync } from 'fflate';

/** Треугольники модели (по 9 чисел), XY-центр габарита в 0, низ на z = 0 — как кладёт пакер. */
export interface ModelMesh {
  positions: Float32Array;
  triangles: number;
  /** Габарит, мм: [x, y, z]. */
  size: [number, number, number];
  /** Растр проекции на стол (считается в воркере разбора; без него сцена строит его сама). */
  projection?: ProjectionMask;
}

export function parseModel(name: string, buffer: ArrayBuffer): ModelMesh {
  const lower = name.toLowerCase();
  if (lower.endsWith('.stl')) return centred(parseStl(buffer));
  if (lower.endsWith('.3mf')) return centred(parse3mf(buffer));
  throw new Error('Поддерживаются .3mf и .stl');
}

function parseStl(buffer: ArrayBuffer): Float32Array {
  const dv = new DataView(buffer);
  if (buffer.byteLength >= 84) {
    const count = dv.getUint32(80, true);
    if (84 + count * 50 === buffer.byteLength) {
      const out = new Float32Array(count * 9);
      for (let i = 0; i < count; i++)
        for (let k = 0; k < 9; k++) out[i * 9 + k] = dv.getFloat32(84 + i * 50 + 12 + k * 4, true);
      return out;
    }
  }
  const text = new TextDecoder().decode(buffer);
  const values: number[] = [];
  for (const m of text.matchAll(/vertex\s+(\S+)\s+(\S+)\s+(\S+)/g)) values.push(+m[1], +m[2], +m[3]);
  return Float32Array.from(values);
}

type Matrix = number[]; // 3MF: 12 чисел, строка-вектор: p' = [x y z 1] · M
const IDENTITY: Matrix = [1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0];

function parseMatrix(text: string | undefined): Matrix {
  const v = (text ?? '').trim().split(/\s+/).map(Number);
  return v.length === 12 && v.every(Number.isFinite) ? v : IDENTITY;
}
function multiply(a: Matrix, b: Matrix): Matrix {
  // a затем b (оба в строковой записи 3MF)
  const r: Matrix = new Array(12).fill(0);
  for (let row = 0; row < 4; row++)
    for (let col = 0; col < 3; col++) {
      let s = row === 3 ? b[9 + col] : 0;
      for (let k = 0; k < 3; k++) s += a[row * 3 + k] * b[k * 3 + col];
      r[row * 3 + col] = s;
    }
  return r;
}
function attrs(tag: string): Record<string, string> {
  const out: Record<string, string> = {};
  for (const m of tag.matchAll(/([\w:]+)="([^"]*)"/g)) out[m[1]] = m[2];
  return out;
}

interface MeshObject {
  vertices: number[];
  triangles: number[];
}
interface ComponentRef {
  objectId: string;
  path: string | null;
  transform: Matrix;
}
interface ModelFile {
  objects: Map<string, MeshObject | ComponentRef[]>;
  items: { objectId: string }[];
}

function parseModelXml(xml: string): ModelFile {
  const objects = new Map<string, MeshObject | ComponentRef[]>();
  for (const m of xml.matchAll(/<object\b([^>]*)>([\s\S]*?)<\/object>/g)) {
    const id = attrs(m[1]).id;
    const body = m[2];
    if (body.includes('<mesh')) {
      const vertices: number[] = [];
      for (const v of body.matchAll(/<vertex\b([^>]*)\/?>/g)) {
        const a = attrs(v[1]);
        vertices.push(+a.x, +a.y, +a.z);
      }
      const triangles: number[] = [];
      for (const t of body.matchAll(/<triangle\b([^>]*)\/?>/g)) {
        const a = attrs(t[1]);
        triangles.push(+a.v1, +a.v2, +a.v3);
      }
      objects.set(id, { vertices, triangles });
    } else {
      const refs: ComponentRef[] = [];
      for (const c of body.matchAll(/<component\b([^>]*)\/?>/g)) {
        const a = attrs(c[1]);
        refs.push({ objectId: a.objectid, path: a['p:path'] ?? null, transform: parseMatrix(a.transform) });
      }
      objects.set(id, refs);
    }
  }
  const items = [...xml.matchAll(/<item\b([^>]*)\/?>/g)].map((m) => ({ objectId: attrs(m[1]).objectid }));
  return { objects, items };
}

/**
 * Первый объект сборки со всеми компонентами (цветные части лежат в 3D/Objects/*.model).
 * Трансформ самого build item не применяется: модель всё равно центрируется на столе.
 */
function parse3mf(buffer: ArrayBuffer): Float32Array {
  const files = unzipSync(new Uint8Array(buffer), { filter: (f) => f.name.toLowerCase().endsWith('.model') });
  const models = new Map<string, ModelFile>();
  for (const [path, data] of Object.entries(files))
    models.set('/' + path.replace(/^\/+/, ''), parseModelXml(strFromU8(data)));
  const rootPath = models.has('/3D/3dmodel.model')
    ? '/3D/3dmodel.model'
    : ([...models.keys()].find((p) => models.get(p)!.items.length) ?? '');
  const root = models.get(rootPath);
  const first = root?.items[0];
  if (!root || !first) throw new Error('В 3MF не найден объект для печати');

  const out: number[] = [];
  const emit = (path: string, objectId: string, m: Matrix, depth: number) => {
    const object = models.get(path)?.objects.get(objectId);
    if (!object || depth > 8) return;
    if (Array.isArray(object)) {
      for (const ref of object) emit(ref.path ?? path, ref.objectId, multiply(ref.transform, m), depth + 1);
      return;
    }
    const v = object.vertices;
    for (const index of object.triangles) {
      const x = v[index * 3],
        y = v[index * 3 + 1],
        z = v[index * 3 + 2];
      out.push(
        x * m[0] + y * m[3] + z * m[6] + m[9],
        x * m[1] + y * m[4] + z * m[7] + m[10],
        x * m[2] + y * m[5] + z * m[8] + m[11]
      );
    }
  };
  emit(rootPath, first.objectId, IDENTITY, 0);
  if (!out.length) throw new Error('В 3MF нет треугольников');
  return Float32Array.from(out);
}

function centred(positions: Float32Array): ModelMesh {
  let minX = Infinity,
    minY = Infinity,
    minZ = Infinity,
    maxX = -Infinity,
    maxY = -Infinity,
    maxZ = -Infinity;
  for (let i = 0; i < positions.length; i += 3) {
    minX = Math.min(minX, positions[i]);
    maxX = Math.max(maxX, positions[i]);
    minY = Math.min(minY, positions[i + 1]);
    maxY = Math.max(maxY, positions[i + 1]);
    minZ = Math.min(minZ, positions[i + 2]);
    maxZ = Math.max(maxZ, positions[i + 2]);
  }
  const cx = (minX + maxX) / 2,
    cy = (minY + maxY) / 2;
  for (let i = 0; i < positions.length; i += 3) {
    positions[i] -= cx;
    positions[i + 1] -= cy;
    positions[i + 2] -= minZ;
  }
  return { positions, triangles: positions.length / 9, size: [maxX - minX, maxY - minY, maxZ - minZ] };
}
