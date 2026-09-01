#!/usr/bin/env python3
"""Clone contour-packed Goat 4 placements into a slicer 3MF template."""

from __future__ import annotations

import argparse
import copy
import io
import json
import math
from pathlib import Path
import uuid
import xml.etree.ElementTree as ET
import zipfile

from PIL import Image, ImageDraw
import trimesh
from shapely.affinity import rotate, translate
from shapely.geometry import Polygon
from shapely.ops import unary_union

CORE = "http://schemas.microsoft.com/3dmanufacturing/core/2015/02"
PRODUCTION = "http://schemas.microsoft.com/3dmanufacturing/production/2015/06"
BAMBU = "http://schemas.bambulab.com/package/2021"
REL = "http://schemas.openxmlformats.org/package/2006/relationships"
NS = {"c": CORE, "p": PRODUCTION}
for prefix, uri in [("", CORE), ("p", PRODUCTION), ("BambuStudio", BAMBU)]:
    ET.register_namespace(prefix, uri)


def _xml(element: ET.Element) -> bytes:
    return ET.tostring(element, encoding="utf-8", xml_declaration=True)


def _stable_uuid(kind: str, index: int) -> str:
    return str(uuid.uuid5(uuid.NAMESPACE_URL, f"goat4-contour-packing/{kind}/{index}"))


def _number(value: float) -> str:
    if abs(value) < 1e-10:
        return "0"
    nearest = round(value)
    if abs(value - nearest) < 1e-10:
        return str(nearest)
    return f"{value:.10f}".rstrip("0").rstrip(".")


def _transform(placement: dict, z_offset: float, footprint_center: tuple[float, float]) -> str:
    # A placement's (x, y) is where the STL mesh's XY bbox center must land in bed coordinates.
    # The build item, however, rotates the object's *local* geometry (embedded mesh plus the
    # source component's own transform — see extract_embedded_stl.py) about the local origin, so
    # the local footprint center has to be rotated out of the translation. Without this, a source
    # component carrying its own translation (Goat 3: (-35.38, +40.12)) displaces every object by
    # a rotation-dependent ~54 mm and objects fall off the heated bed.
    angle = math.radians(float(placement["angle_deg"]))
    c, s = math.cos(angle), math.sin(angle)
    center_x, center_y = footprint_center
    translation = [
        float(placement["x_mm"]) - (center_x * c - center_y * s),
        float(placement["y_mm"]) - (center_x * s + center_y * c),
        z_offset,
    ]
    values = [c, s, 0.0, -s, c, 0.0, 0.0, 0.0, 1.0]
    return " ".join(_number(v) for v in values) + " " + " ".join(_number(v) for v in translation)


def _lower_polygon(stl_path: Path):
    mesh = trimesh.load_mesh(stl_path, process=False)
    min_z = float(mesh.vertices[:, 2].min())
    bottom = mesh.faces[(abs(mesh.vertices[mesh.faces][:, :, 2] - min_z) < 1e-5).all(axis=1)]
    polygon = unary_union([Polygon(mesh.vertices[face][:, :2]) for face in bottom])
    center_x = (float(mesh.vertices[:, 0].min()) + float(mesh.vertices[:, 0].max())) * 0.5
    center_y = (float(mesh.vertices[:, 1].min()) + float(mesh.vertices[:, 1].max())) * 0.5
    return translate(polygon, -center_x, -center_y), (center_x, center_y)


def _parse_xy_polygon(values: list[str]) -> list[tuple[float, float]]:
    result = []
    for value in values:
        x, y = value.split("x", 1)
        result.append((float(x), float(y)))
    return result


def _placed_polygons(stl_path: Path, placements: list[dict]):
    base, center = _lower_polygon(stl_path)
    polygons = [
        translate(
            rotate(base, float(p["angle_deg"]), origin=(0.0, 0.0), use_radians=False),
            float(p["x_mm"]),
            float(p["y_mm"]),
        )
        for p in placements
    ]
    return polygons, center


def _draw_preview(
    polygons,
    printable: list[tuple[float, float]],
    exclusions: list[list[tuple[float, float]]],
    size: int,
    pick: bool = False,
    edge_margin: float = 0.0,
    purge_lines: list[tuple[float, float, float, float, float]] | None = None,
    custom_keepouts: list[tuple[float, float, float, float]] | None = None,
) -> bytes:
    image = Image.new("RGBA", (size, size), (245, 247, 250, 255))
    draw = ImageDraw.Draw(image)
    min_x = min(x for x, _ in printable)
    max_x = max(x for x, _ in printable)
    min_y = min(y for _, y in printable)
    max_y = max(y for _, y in printable)
    margin = max(3, round(size * 0.03))
    scale = min((size - 2 * margin) / (max_x - min_x), (size - 2 * margin) / (max_y - min_y))

    def screen(point):
        x, y = point
        return (
            margin + (x - min_x) * scale,
            size - margin - (y - min_y) * scale,
        )

    draw.polygon([screen(p) for p in printable], fill=(224, 228, 233, 255), outline=(45, 50, 58, 255), width=max(1, size // 256))
    if edge_margin > 0:
        safe_outline = [
            (min_x + edge_margin, min_y + edge_margin),
            (max_x - edge_margin, min_y + edge_margin),
            (max_x - edge_margin, max_y - edge_margin),
            (min_x + edge_margin, max_y - edge_margin),
            (min_x + edge_margin, min_y + edge_margin),
        ]
        draw.line(
            [screen(point) for point in safe_outline],
            fill=(35, 170, 95, 255),
            width=max(1, size // 256),
        )
    for x0, y0, x1, y1, width in purge_lines or []:
        draw.line(
            [screen((x0, y0)), screen((x1, y1))],
            fill=(230, 72, 72, 255),
            width=max(2, round(width * scale)),
        )
    for exclusion in exclusions:
        if len(exclusion) >= 3:
            draw.polygon([screen(p) for p in exclusion], fill=(242, 112, 112, 230))
    for min_x_keepout, min_y_keepout, max_x_keepout, max_y_keepout in custom_keepouts or []:
        first = screen((min_x_keepout, min_y_keepout))
        second = screen((max_x_keepout, max_y_keepout))
        draw.rectangle(
            [(min(first[0], second[0]), min(first[1], second[1])), (max(first[0], second[0]), max(first[1], second[1]))],
            fill=(238, 140, 76, 170),
            outline=(185, 82, 26, 255),
        )
    for index, geometry in enumerate(polygons):
        if geometry.geom_type == "Polygon":
            parts = [geometry]
        else:
            parts = geometry.geoms
        if pick:
            fill = ((37 * (index + 1)) % 254 + 1, (83 * (index + 1)) % 254 + 1, (149 * (index + 1)) % 254 + 1, 255)
        else:
            fill = (111, 169, 220, 255)
        for part in parts:
            draw.polygon([screen(p) for p in part.exterior.coords], fill=fill, outline=(22, 91, 148, 255))
            for hole in part.interiors:
                draw.polygon([screen(p) for p in hole.coords], fill=(224, 228, 233, 255))
    output = io.BytesIO()
    image.save(output, format="PNG", optimize=True)
    return output.getvalue()


def _rebuild_core(
    data: bytes,
    placements: list[dict],
    component_paths: list[str] | None = None,
    reuse_source_object_instances: bool = False,
    inline_mesh_template: ET.Element | None = None,
    z_offset: float = 3.70000005,
    footprint_center: tuple[float, float] = (0.0, 0.0),
) -> bytes:
    root = ET.fromstring(data)
    resources = root.find("c:resources", NS)
    build = root.find("c:build", NS)
    if resources is None or build is None:
        raise ValueError("3MF core model lacks resources/build")
    original_objects = resources.findall("c:object", NS)
    if not original_objects:
        raise ValueError("3MF core model has no source object")
    template_object = original_objects[0]
    source_object_id = template_object.get("id")
    if inline_mesh_template is not None:
        for child in list(resources):
            resources.remove(child)
    elif reuse_source_object_instances:
        for child in list(resources):
            if child is not template_object:
                resources.remove(child)
    else:
        for child in list(resources):
            resources.remove(child)
    for child in list(build):
        build.remove(child)
    for index, placement in enumerate(placements):
        object_id = source_object_id if reuse_source_object_instances else str(index + 2)
        if inline_mesh_template is not None:
            obj = copy.deepcopy(inline_mesh_template)
            obj.set("id", object_id)
            obj.set(f"{{{PRODUCTION}}}UUID", _stable_uuid("inline-resource", index))
            resources.append(obj)
        elif not reuse_source_object_instances:
            obj = copy.deepcopy(template_object)
            obj.set("id", object_id)
            obj.set(f"{{{PRODUCTION}}}UUID", _stable_uuid("resource", index))
            component = obj.find(".//c:component", NS)
            if component is not None:
                component.set(f"{{{PRODUCTION}}}UUID", _stable_uuid("component", index))
                if component_paths:
                    component.set(f"{{{PRODUCTION}}}path", component_paths[index])
            resources.append(obj)
        item = ET.SubElement(build, f"{{{CORE}}}item")
        item.set("objectid", object_id)
        item.set(f"{{{PRODUCTION}}}UUID", _stable_uuid("build-item", index))
        item.set("transform", _transform(placement, z_offset, footprint_center))
        item.set("printable", "1")
    return _xml(root)


def _rebuild_model_settings(
    data: bytes,
    placements: list[dict],
    reuse_source_object_instances: bool = False,
    z_offset: float = 3.70000005,
) -> bytes:
    root = ET.fromstring(data)
    object_nodes = root.findall("object")
    if not object_nodes:
        raise ValueError("model_settings has no object template")
    template = object_nodes[0]
    source_object_id = template.get("id")
    if not reuse_source_object_instances:
        insertion = list(root).index(template)
        for node in object_nodes:
            root.remove(node)
        for index, _placement in enumerate(placements):
            object_id = index + 2
            node = copy.deepcopy(template)
            node.set("id", str(object_id))
            for part_index, part in enumerate(node.findall("part")):
                part.set("uuid", _stable_uuid(f"part-{part_index}", index))
            root.insert(insertion + index, node)
    if not reuse_source_object_instances:
        plate = root.find("plate")
        if plate is None:
            raise ValueError("model_settings has no plate")
        for instance in plate.findall("model_instance"):
            plate.remove(instance)
        for index, _placement in enumerate(placements):
            instance = ET.SubElement(plate, "model_instance")
            for key, value in [
                ("object_id", str(index + 2)),
                ("instance_id", "0"),
                ("identify_id", str(1000 + index * 10)),
            ]:
                ET.SubElement(instance, "metadata", key=key, value=value)
        assemble = root.find("assemble")
        if assemble is not None:
            for child in list(assemble):
                assemble.remove(child)
            for index, _placement in enumerate(placements):
                ET.SubElement(
                    assemble,
                    "assemble_item",
                    object_id=str(index + 2),
                    instance_id="0",
                    transform=f"1 0 0 0 1 0 0 0 1 0 0 {_number(z_offset)}",
                    offset="0 0 0",
                )
    return _xml(root)


def _rebuild_cut_information(data: bytes, count: int) -> bytes:
    root = ET.fromstring(data)
    for child in list(root):
        root.remove(child)
    for index in range(count):
        obj = ET.SubElement(root, "object", id=str(index + 1))
        ET.SubElement(obj, "cut_id", id="0", check_sum="1", connectors_cnt="0")
    return _xml(root)


def _duplicate_colored_mesh_parts(entries: dict[str, bytes], core_data: bytes, count: int) -> list[str]:
    root = ET.fromstring(core_data)
    component = root.find(".//c:resources/c:object/c:components/c:component", NS)
    if component is None:
        raise ValueError("3MF core model has no source component")
    source_path = component.get(f"{{{PRODUCTION}}}path")
    if not source_path:
        raise ValueError("source component has no production path")
    source_entry = source_path.lstrip("/")
    if source_entry not in entries:
        raise ValueError(f"source mesh part missing from archive: {source_entry}")
    paths = [source_path]
    if count == 1:
        return paths
    base = source_entry.removesuffix(".model")
    relationship_path = "3D/_rels/3dmodel.model.rels"
    relationship_text = entries.get(relationship_path, b"").decode("utf-8")
    for index in range(1, count):
        duplicate_entry = f"{base}_instance_{index + 1}.model"
        entries[duplicate_entry] = entries[source_entry]
        duplicate_path = "/" + duplicate_entry
        paths.append(duplicate_path)
        if relationship_text:
            relationship = (
                f' <Relationship Target="{duplicate_path}" Id="paint-copy-{index + 1}" '
                'Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/>\n'
            )
            relationship_text = relationship_text.replace("</Relationships>", relationship + "</Relationships>")
    if relationship_text:
        entries[relationship_path] = relationship_text.encode("utf-8")
    return paths


def _parse_transform(text: str | None) -> list[list[float]]:
    """Parse a 3MF transform into a 4x3 row-major matrix (last row = translation).

    3MF uses row vectors: p' = p·M, i.e. p'_j = sum_i p_i * M[i][j] + M[3][j].
    """
    if not text:
        return [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [0.0, 0.0, 0.0]]
    values = [float(value) for value in text.split()]
    if len(values) != 12:
        raise ValueError(f"3MF transform is not 12 numbers: {text!r}")
    return [values[0:3], values[3:6], values[6:9], values[9:12]]


def _load_inline_mesh_template(entries: dict[str, bytes], core_data: bytes) -> ET.Element:
    root = ET.fromstring(core_data)
    component = root.find(".//c:resources/c:object/c:components/c:component", NS)
    if component is None:
        raise ValueError("3MF core model has no source component")
    source_path = component.get(f"{{{PRODUCTION}}}path")
    if not source_path:
        raise ValueError("source component has no production path")
    source_entry = source_path.lstrip("/")
    mesh_root = ET.fromstring(entries[source_entry])
    mesh_object = mesh_root.find(".//c:resources/c:object", NS)
    if mesh_object is None or mesh_object.find("c:mesh", NS) is None:
        raise ValueError("source component has no inline mesh object")
    # An inlined mesh replaces the component, so the component's transform — normally applied
    # between mesh and build item — must be baked into the vertex coordinates to keep the
    # object-local geometry identical to the component-based modes (see _transform).
    matrix = _parse_transform(component.get("transform"))
    for vertex in mesh_object.findall(".//c:vertex", NS):
        point = (float(vertex.get("x")), float(vertex.get("y")), float(vertex.get("z")))
        x, y, z = (
            point[0] * matrix[0][j] + point[1] * matrix[1][j] + point[2] * matrix[2][j] + matrix[3][j]
            for j in range(3)
        )
        vertex.set("x", _number(x))
        vertex.set("y", _number(y))
        vertex.set("z", _number(z))
    return mesh_object


def _source_z_offset(core_data: bytes) -> float:
    root = ET.fromstring(core_data)
    item = root.find(".//c:build/c:item", NS)
    if item is None or not item.get("transform"):
        raise ValueError("3MF source build item has no transform")
    values = item.get("transform").split()
    if len(values) != 12:
        raise ValueError("3MF source transform is not a 3x4 matrix")
    return float(values[11])


def build_project(
    template: Path,
    placements_json: Path,
    output: Path,
    stl_path: Path | None = None,
    edge_margin: float = 0.0,
    purge_lines: list[tuple[float, float, float, float, float]] | None = None,
    custom_keepouts: list[tuple[float, float, float, float]] | None = None,
    duplicate_mesh_per_object: bool = False,
    reuse_source_object_instances: bool = False,
    inline_colored_mesh_objects: bool = False,
) -> None:
    template = Path(template)
    placements_json = Path(placements_json)
    output = Path(output)
    if stl_path:
        stl_path = Path(stl_path)
    else:
        candidates = [
            template.parent / "goat4_v2.stl",
            template.parent.parent / "source" / "goat4_v2.stl",
        ]
        stl_path = next((candidate for candidate in candidates if candidate.exists()), candidates[0])
    placement_data = json.loads(placements_json.read_text())
    placements = placement_data["placements"]
    if not placements:
        raise ValueError("placement list is empty")
    with zipfile.ZipFile(template) as archive:
        entries = {name: archive.read(name) for name in archive.namelist()}
    settings = json.loads(entries["Metadata/project_settings.config"])
    printable = _parse_xy_polygon(settings["printable_area"])
    raw_exclusion = settings.get("bed_exclude_area", [])
    exclusions = []
    if len(raw_exclusion) >= 3:
        exclusions.append(_parse_xy_polygon(raw_exclusion))
    polygons, footprint_center = _placed_polygons(stl_path, placements)
    z_offset = _source_z_offset(entries["3D/3dmodel.model"])

    component_paths = None
    if duplicate_mesh_per_object:
        component_paths = _duplicate_colored_mesh_parts(
            entries, entries["3D/3dmodel.model"], len(placements)
        )
    inline_mesh_template = None
    if inline_colored_mesh_objects:
        inline_mesh_template = _load_inline_mesh_template(entries, entries["3D/3dmodel.model"])
    entries["3D/3dmodel.model"] = _rebuild_core(
        entries["3D/3dmodel.model"],
        placements,
        component_paths,
        reuse_source_object_instances,
        inline_mesh_template,
        z_offset,
        footprint_center,
    )
    if not reuse_source_object_instances:
        entries["Metadata/model_settings.config"] = _rebuild_model_settings(
            entries["Metadata/model_settings.config"], placements, z_offset=z_offset
        )
    if "Metadata/cut_information.xml" in entries and not reuse_source_object_instances:
        entries["Metadata/cut_information.xml"] = _rebuild_cut_information(
            entries["Metadata/cut_information.xml"], len(placements)
        )
    if "Metadata/plate_1.json" in entries and not reuse_source_object_instances:
        plate = json.loads(entries["Metadata/plate_1.json"])
        bounds = [geometry.bounds for geometry in polygons]
        plate["bbox_all"] = [
            min(b[0] for b in bounds),
            min(b[1] for b in bounds),
            max(b[2] for b in bounds),
            max(b[3] for b in bounds),
        ]
        plate["bbox_objects"] = [
            {
                "area": geometry.area,
                "bbox": list(geometry.bounds),
                "id": 1002 + index * 10,
                "layer_height": 0.2,
                "name": "goat4_v2.stl",
            }
            for index, geometry in enumerate(polygons)
        ]
        plate["first_layer_time"] = 0.0
        entries["Metadata/plate_1.json"] = json.dumps(plate, separators=(",", ":")).encode()

    plate_png = _draw_preview(
        polygons,
        printable,
        exclusions,
        512,
        edge_margin=edge_margin,
        purge_lines=purge_lines,
        custom_keepouts=custom_keepouts,
    )
    pick_png = _draw_preview(
        polygons,
        printable,
        exclusions,
        512,
        pick=True,
        edge_margin=edge_margin,
        purge_lines=purge_lines,
        custom_keepouts=custom_keepouts,
    )
    entries["Metadata/plate_1.png"] = plate_png
    entries["Metadata/plate_no_light_1.png"] = plate_png
    entries["Metadata/top_1.png"] = plate_png
    entries["Metadata/pick_1.png"] = pick_png
    entries["Metadata/plate_1_small.png"] = _draw_preview(
        polygons,
        printable,
        exclusions,
        128,
        edge_margin=edge_margin,
        purge_lines=purge_lines,
        custom_keepouts=custom_keepouts,
    )

    output.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(output, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=6) as archive:
        for name, data in entries.items():
            archive.writestr(name, data)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("template", type=Path)
    parser.add_argument("placements", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--stl", type=Path)
    parser.add_argument("--edge-margin", type=float, default=0.0)
    parser.add_argument("--purge-line", action="append", default=[])
    parser.add_argument("--keepout", action="append", default=[])
    parser.add_argument("--duplicate-mesh-per-object", action="store_true")
    parser.add_argument("--reuse-source-object-instances", action="store_true")
    parser.add_argument("--inline-colored-mesh-objects", action="store_true")
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
    build_project(
        args.template,
        args.placements,
        args.output,
        args.stl,
        args.edge_margin,
        purge_lines,
        custom_keepouts,
        args.duplicate_mesh_per_object,
        args.reuse_source_object_instances,
        args.inline_colored_mesh_objects,
    )
    print(args.output)


if __name__ == "__main__":
    main()
