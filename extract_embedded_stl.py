#!/usr/bin/env python3
"""Export a source 3MF's single embedded object mesh to a binary STL.

The packer must collide against the exact geometry that will actually be printed. A
separately-exported STL for the same model can drift from the 3MF's embedded mesh (different
export pass, different tool version, manual edits), silently invalidating collision checks that
assume the two are identical. Deriving the STL directly from the 3MF's own component eliminates
that class of mismatch.
"""

from __future__ import annotations

import argparse
import struct
import xml.etree.ElementTree as ET
import zipfile
from pathlib import Path

CORE = "http://schemas.microsoft.com/3dmanufacturing/core/2015/02"
PRODUCTION = "http://schemas.microsoft.com/3dmanufacturing/production/2015/06"
NS = {"c": CORE, "p": PRODUCTION}


def embedded_mesh_triangles(source_3mf: Path) -> list[tuple[tuple[float, float, float], ...]]:
    with zipfile.ZipFile(source_3mf) as archive:
        core = ET.fromstring(archive.read("3D/3dmodel.model"))
        component = core.find(".//c:resources/c:object/c:components/c:component", NS)
        if component is not None:
            source_path = component.get(f"{{{PRODUCTION}}}path")
            if not source_path:
                raise ValueError("source component has no production path")
            mesh_root = ET.fromstring(archive.read(source_path.lstrip("/")))
        else:
            mesh_root = core
        mesh_object = mesh_root.find(".//c:resources/c:object", NS)
        if mesh_object is None or mesh_object.find("c:mesh", NS) is None:
            raise ValueError("3MF has no inline or component mesh to export")
        vertices = [
            (float(v.get("x")), float(v.get("y")), float(v.get("z")))
            for v in mesh_object.findall(".//c:vertex", NS)
        ]
        triangles = []
        for triangle in mesh_object.findall(".//c:triangle", NS):
            v1, v2, v3 = (int(triangle.get(key)) for key in ("v1", "v2", "v3"))
            triangles.append((vertices[v1], vertices[v2], vertices[v3]))
        return triangles


def write_binary_stl(path: Path, triangles: list[tuple[tuple[float, float, float], ...]]) -> None:
    with path.open("wb") as handle:
        handle.write(b"\x00" * 80)
        handle.write(struct.pack("<I", len(triangles)))
        for triangle in triangles:
            handle.write(struct.pack("<3f", 0.0, 0.0, 0.0))
            for vertex in triangle:
                handle.write(struct.pack("<3f", *vertex))
            handle.write(struct.pack("<H", 0))


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("source_3mf", type=Path)
    parser.add_argument("output_stl", type=Path)
    args = parser.parse_args()
    triangles = embedded_mesh_triangles(args.source_3mf)
    if not triangles:
        raise SystemExit("embedded mesh has no triangles")
    args.output_stl.parent.mkdir(parents=True, exist_ok=True)
    write_binary_stl(args.output_stl, triangles)
    print(args.output_stl)


if __name__ == "__main__":
    main()
