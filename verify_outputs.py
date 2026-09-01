#!/usr/bin/env python3
"""Independent exact-geometry and 3MF integrity checks for packed projects."""

from __future__ import annotations

import argparse
import json
import math
from pathlib import Path
import xml.etree.ElementTree as ET
import zipfile

import trimesh
from shapely.affinity import rotate, translate
from shapely.geometry import LineString, Polygon
from shapely.ops import unary_union

CORE = "http://schemas.microsoft.com/3dmanufacturing/core/2015/02"
PRODUCTION = "http://schemas.microsoft.com/3dmanufacturing/production/2015/06"
NS = {"c": CORE, "p": PRODUCTION}


def parse_xy(values):
    return [(float(value.split("x")[0]), float(value.split("x")[1])) for value in values]


def lower_polygon(stl_path: Path):
    mesh = trimesh.load_mesh(stl_path, process=False)
    z = mesh.vertices[:, 2].min()
    bottom = mesh.faces[(abs(mesh.vertices[mesh.faces][:, :, 2] - z) < 1e-5).all(axis=1)]
    polygon = unary_union([Polygon(mesh.vertices[face][:, :2]) for face in bottom])
    cx = (mesh.vertices[:, 0].min() + mesh.vertices[:, 0].max()) * 0.5
    cy = (mesh.vertices[:, 1].min() + mesh.vertices[:, 1].max()) * 0.5
    return translate(polygon, -cx, -cy)


def stl_bbox_center(stl_path: Path) -> tuple[float, float]:
    mesh = trimesh.load_mesh(stl_path, process=False)
    return (
        (float(mesh.vertices[:, 0].min()) + float(mesh.vertices[:, 0].max())) * 0.5,
        (float(mesh.vertices[:, 1].min()) + float(mesh.vertices[:, 1].max())) * 0.5,
    )


def parse_transform(text):
    """Parse a 3MF transform into a 4x3 row-major matrix (last row = translation).

    3MF uses row vectors: p' = p·M, i.e. p'_j = sum_i p_i * M[i][j] + M[3][j].
    """
    if not text:
        return [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [0.0, 0.0, 0.0]]
    values = [float(value) for value in text.split()]
    if len(values) != 12:
        raise AssertionError(f"3MF transform is not 12 numbers: {text!r}")
    return [values[0:3], values[3:6], values[6:9], values[9:12]]


def apply_matrix(matrix, point):
    x, y, z = point
    return tuple(
        x * matrix[0][j] + y * matrix[1][j] + z * matrix[2][j] + matrix[3][j]
        for j in range(3)
    )


def verify(
    stl_path: Path,
    placements_path: Path,
    project_path: Path,
    minimum_gap: float = 0.5,
    edge_margin: float = 0.0,
    purge_lines: list[tuple[float, float, float, float, float]] | None = None,
    purge_gap: float = 0.5,
    custom_keepouts: list[tuple[float, float, float, float]] | None = None,
):
    placement_data = json.loads(placements_path.read_text())
    placements = placement_data["placements"]
    base = lower_polygon(stl_path)
    polygons = [
        translate(rotate(base, p["angle_deg"], origin=(0, 0)), p["x_mm"], p["y_mm"])
        for p in placements
    ]
    with zipfile.ZipFile(project_path) as archive:
        bad_member = archive.testzip()
        if bad_member:
            raise AssertionError(f"ZIP CRC failed: {bad_member}")
        settings = json.loads(archive.read("Metadata/project_settings.config"))
        printable = Polygon(parse_xy(settings["printable_area"]))
        safe_printable = printable.buffer(-edge_margin, join_style=2)
        if safe_printable.is_empty:
            raise AssertionError(f"edge margin {edge_margin} mm consumes printable area")
        raw_exclusion = settings.get("bed_exclude_area", [])
        exclusion = Polygon(parse_xy(raw_exclusion)) if len(raw_exclusion) >= 3 else None
        root = ET.fromstring(archive.read("3D/3dmodel.model"))
        resources = root.findall(".//c:resources/c:object", NS)
        items = root.findall(".//c:build/c:item", NS)
        model_settings = ET.fromstring(archive.read("Metadata/model_settings.config"))
        config_objects = model_settings.findall("object")
        instances = model_settings.findall("plate/model_instance")
        if len(items) != len(placements):
            raise AssertionError(
                f"object-count mismatch: placements={len(placements)}, resources={len(resources)}, "
                f"items={len(items)}, config={len(config_objects)}, instances={len(instances)}"
            )
        if len(resources) not in (1, len(placements)) or len(config_objects) not in (1, len(placements)) or len(instances) not in (1, len(placements)):
            raise AssertionError(
                f"unsupported resource/config/instance cardinality: resources={len(resources)}, "
                f"config={len(config_objects)}, instances={len(instances)}"
            )
        resource_ids = {node.get("id") for node in resources}
        if resource_ids != {item.get("objectid") for item in items}:
            raise AssertionError("build items do not reference exactly the resource objects")
        paths = [component.get(f"{{{PRODUCTION}}}path").lstrip("/") for component in root.findall(".//c:component", NS)]
        if any(path not in archive.namelist() for path in paths):
            raise AssertionError("a component path is missing from the archive")
        source_model = ET.fromstring(archive.read(paths[0]))
        embedded_vertices = [
            (float(v.get("x")), float(v.get("y")), float(v.get("z")))
            for v in source_model.findall(".//c:vertex", NS)
        ]
        if not embedded_vertices:
            raise AssertionError("embedded component mesh has no vertices")
        # The component's own transform sits between the embedded mesh and every build item
        # (world = mesh ∘ component ∘ item). It can carry a translation (Goat 3:
        # (-35.38, +40.12, -4.3)); ignoring it makes both the XY and Z placement checks lie.
        component = root.find(".//c:resources/c:object/c:components/c:component", NS)
        component_matrix = parse_transform(component.get("transform") if component is not None else None)
        local_vertices = [apply_matrix(component_matrix, vertex) for vertex in embedded_vertices]
        embedded_height = max(v[2] for v in local_vertices) - min(v[2] for v in local_vertices)
        for index, (item, expected) in enumerate(zip(items, placements)):
            matrix = list(map(float, item.get("transform").split()))
            if len(matrix) != 12:
                raise AssertionError(f"item {index} transform is not 3x4")
            angle = math.degrees(math.atan2(matrix[1], matrix[0])) % 360.0
            if abs(((angle - expected["angle_deg"] + 180) % 360) - 180) > 1e-6:
                raise AssertionError(f"item {index} angle mismatch: {angle} != {expected['angle_deg']}")
            # The item translation places the object-local footprint center (embedded mesh plus
            # the source component's transform) at (x_mm, y_mm), so the expected translation is
            # the placement minus the angle-rotated STL bbox center — the same compensation
            # build_3mf.py applies. Comparing raw matrix[9]/[10] to the placement would only hold
            # for a component whose own translation is zero.
            radians = math.radians(expected["angle_deg"])
            center_x, center_y = stl_bbox_center(stl_path)
            expected_tx = expected["x_mm"] - (center_x * math.cos(radians) - center_y * math.sin(radians))
            expected_ty = expected["y_mm"] - (center_x * math.sin(radians) + center_y * math.cos(radians))
            if abs(matrix[9] - expected_tx) > 1e-6 or abs(matrix[10] - expected_ty) > 1e-6:
                raise AssertionError(
                    f"item {index} XY translation mismatch: ({matrix[9]}, {matrix[10]}) != "
                    f"({expected_tx}, {expected_ty})"
                )
            item_matrix = parse_transform(item.get("transform"))
            transformed_z = [
                apply_matrix(item_matrix, vertex)[2] for vertex in local_vertices
            ]
            if abs(min(transformed_z)) > 1e-5 or abs(max(transformed_z) - embedded_height) > 1e-4:
                raise AssertionError(f"item {index} is not on bed: z={min(transformed_z)}..{max(transformed_z)}")

    edge_clearances = []
    for index, polygon in enumerate(polygons):
        if not safe_printable.covers(polygon):
            raise AssertionError(
                f"placement {index} violates {edge_margin:.3f} mm printable-area margin"
            )
        edge_clearances.append(polygon.distance(printable.boundary))
        if exclusion is not None and polygon.intersection(exclusion).area > 1e-7:
            raise AssertionError(f"placement {index} enters exclusion polygon")
        for keepout in custom_keepouts or []:
            keepout_polygon = Polygon([
                (keepout[0], keepout[1]),
                (keepout[2], keepout[1]),
                (keepout[2], keepout[3]),
                (keepout[0], keepout[3]),
            ])
            if polygon.intersection(keepout_polygon).area > 1e-7:
                raise AssertionError(f"placement {index} enters custom keepout")
    distances = [
        polygons[i].distance(polygons[j])
        for i in range(len(polygons))
        for j in range(i)
    ]
    minimum_distance = min(distances) if distances else math.inf
    if minimum_distance + 1e-6 < minimum_gap:
        raise AssertionError(f"minimum exact gap {minimum_distance:.6f} mm is below {minimum_gap:.6f} mm")
    purge_geometries = [
        LineString([(x0, y0), (x1, y1)]).buffer(width * 0.5, cap_style=2, join_style=2)
        for x0, y0, x1, y1, width in (purge_lines or [])
    ]
    purge_distances = [
        polygon.distance(purge)
        for polygon in polygons
        for purge in purge_geometries
    ]
    minimum_purge_distance = min(purge_distances) if purge_distances else math.inf
    if minimum_purge_distance + 1e-6 < purge_gap:
        raise AssertionError(
            f"minimum purge-line gap {minimum_purge_distance:.6f} mm is below {purge_gap:.6f} mm"
        )
    return {
        "project": str(project_path),
        "printer_model": settings.get("printer_model"),
        "count": len(placements),
        "minimum_exact_gap_mm": minimum_distance,
        "required_edge_margin_mm": edge_margin,
        "minimum_edge_clearance_mm": min(edge_clearances),
        "required_purge_gap_mm": purge_gap if purge_geometries else None,
        "minimum_purge_gap_mm": minimum_purge_distance if purge_geometries else None,
        "all_inside_printable_area": True,
        "exclusion_clear": True,
        "zip_crc_ok": True,
        "metadata_object_counts_match": True,
        "embedded_mesh_transform_ok": True,
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("stl", type=Path)
    parser.add_argument("placements", type=Path)
    parser.add_argument("project", type=Path)
    parser.add_argument("--minimum-gap", type=float, default=0.5)
    parser.add_argument("--edge-margin", type=float, default=0.0)
    parser.add_argument(
        "--purge-line",
        action="append",
        default=[],
        help="axis-aligned segment x0,y0,x1,y1,width; repeatable",
    )
    parser.add_argument("--purge-gap", type=float, default=0.5)
    parser.add_argument("--keepout", action="append", default=[])
    parser.add_argument("--report", type=Path)
    args = parser.parse_args()
    purge_lines = []
    for value in args.purge_line:
        parts = tuple(map(float, value.split(',')))
        if len(parts) != 5:
            parser.error("--purge-line must be x0,y0,x1,y1,width")
        purge_lines.append(parts)
    custom_keepouts = []
    for value in args.keepout:
        parts = tuple(map(float, value.split(',')))
        if len(parts) != 4:
            parser.error("--keepout must be min_x,min_y,max_x,max_y")
        custom_keepouts.append(parts)
    report = verify(
        args.stl,
        args.placements,
        args.project,
        args.minimum_gap,
        args.edge_margin,
        purge_lines,
        args.purge_gap,
        custom_keepouts,
    )
    text = json.dumps(report, ensure_ascii=False, indent=2)
    print(text)
    if args.report:
        args.report.write_text(text + "\n")


if __name__ == "__main__":
    main()
