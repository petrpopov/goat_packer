import importlib.util
import io
import json
import math
from pathlib import Path
import tempfile
import unittest
import xml.etree.ElementTree as ET
import zipfile

from PIL import Image
import trimesh

ROOT = Path(__file__).resolve().parents[2]
MODULE_PATH = ROOT / "goat_packer" / "build_3mf.py"
VERIFY_MODULE_PATH = ROOT / "goat_packer" / "verify_outputs.py"
EXTRACT_MODULE_PATH = ROOT / "goat_packer" / "extract_embedded_stl.py"

CORE_NS = {"c": "http://schemas.microsoft.com/3dmanufacturing/core/2015/02"}


class Build3mfTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        spec = importlib.util.spec_from_file_location("build_3mf", MODULE_PATH)
        cls.module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cls.module)
        verify_spec = importlib.util.spec_from_file_location("verify_outputs", VERIFY_MODULE_PATH)
        cls.verify_module = importlib.util.module_from_spec(verify_spec)
        verify_spec.loader.exec_module(cls.verify_module)
        extract_spec = importlib.util.spec_from_file_location("extract_embedded_stl", EXTRACT_MODULE_PATH)
        cls.extract_module = importlib.util.module_from_spec(extract_spec)
        extract_spec.loader.exec_module(cls.extract_module)

    def placements_file(self, directory: Path) -> Path:
        path = directory / "placements.json"
        path.write_text(
            json.dumps(
                {
                    "placements": [
                        {"x_mm": 60.0, "y_mm": 70.0, "angle_deg": 0.0},
                        {"x_mm": 150.0, "y_mm": 160.0, "angle_deg": 90.0},
                    ],
                    "resolution_mm": 0.25,
                    "requested_clearance_mm": 0.7,
                    "angle_step_deg": 2.0,
                    "attempts": 10,
                    "seed": 1,
                }
            )
        )
        return path

    def resolved_stl(self, source: Path) -> Path:
        # Mirror build_project's own fallback when no --stl is passed.
        candidates = [
            source.parent / "goat4_v2.stl",
            source.parent.parent / "source" / "goat4_v2.stl",
        ]
        return next((candidate for candidate in candidates if candidate.exists()), candidates[0])

    def expected_item_transform(self, placement: dict, center: tuple[float, float], z_offset: float) -> list[float]:
        angle = math.radians(placement["angle_deg"])
        c, s = math.cos(angle), math.sin(angle)
        return [
            c, s, 0.0, -s, c, 0.0, 0.0, 0.0, 1.0,
            placement["x_mm"] - (center[0] * c - center[1] * s),
            placement["y_mm"] - (center[0] * s + center[1] * c),
            z_offset,
        ]

    def assert_transform_matches(self, transform_text: str, expected: list[float]):
        actual = [float(value) for value in transform_text.split()]
        self.assertEqual(len(actual), 12)
        for value, reference in zip(actual, expected):
            self.assertAlmostEqual(value, reference, places=6)

    def assert_common_integrity(self, source: Path, output: Path):
        core_ns = {"c": "http://schemas.microsoft.com/3dmanufacturing/core/2015/02"}
        stl = self.resolved_stl(source)
        mesh = trimesh.load_mesh(stl, process=False)
        center = (
            (float(mesh.vertices[:, 0].min()) + float(mesh.vertices[:, 0].max())) * 0.5,
            (float(mesh.vertices[:, 1].min()) + float(mesh.vertices[:, 1].max())) * 0.5,
        )
        with zipfile.ZipFile(source) as original, zipfile.ZipFile(output) as built:
            self.assertIsNone(built.testzip())
            self.assertEqual(
                original.read("Metadata/project_settings.config"),
                built.read("Metadata/project_settings.config"),
            )
            root = ET.fromstring(built.read("3D/3dmodel.model"))
            self.assertEqual(len(root.findall(".//c:resources/c:object", core_ns)), 2)
            items = root.findall(".//c:build/c:item", core_ns)
            self.assertEqual(len(items), 2)
            self.assert_transform_matches(items[0].get("transform"), self.expected_item_transform({"x_mm": 60.0, "y_mm": 70.0, "angle_deg": 0.0}, center, 3.70000005))
            self.assert_transform_matches(items[1].get("transform"), self.expected_item_transform({"x_mm": 150.0, "y_mm": 160.0, "angle_deg": 90.0}, center, 3.70000005))
            settings = ET.fromstring(built.read("Metadata/model_settings.config"))
            self.assertEqual(len(settings.findall("object")), 2)
            self.assertEqual(len(settings.findall("plate/model_instance")), 2)
            relationships = ET.fromstring(built.read("3D/_rels/3dmodel.model.rels"))
            rel_ns = {"r": "http://schemas.openxmlformats.org/package/2006/relationships"}
            for rel in relationships.findall("r:Relationship", rel_ns):
                target = rel.get("Target").lstrip("/")
                self.assertIn(target, built.namelist())
            for name, size in {
                "Metadata/plate_1.png": (512, 512),
                "Metadata/plate_1_small.png": (128, 128),
                "Metadata/plate_no_light_1.png": (512, 512),
                "Metadata/top_1.png": (512, 512),
                "Metadata/pick_1.png": (512, 512),
            }.items():
                self.assertEqual(Image.open(io.BytesIO(built.read(name))).size, size)

    def test_bambu_build_updates_optional_bambu_metadata(self):
        source = (
            ROOT
            / "Goat 4"
            / "p1s"
            / "goat4_p1s_contour_packed_purge_safe_faster.3mf"
        )
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            output = tmp / "bambu.3mf"
            self.module.build_project(source, self.placements_file(tmp), output)
            self.assert_common_integrity(source, output)
            with zipfile.ZipFile(output) as built:
                cut = ET.fromstring(built.read("Metadata/cut_information.xml"))
                self.assertEqual(len(cut.findall("object")), 2)
                plate = json.loads(built.read("Metadata/plate_1.json"))
                self.assertEqual(len(plate["bbox_objects"]), 2)

    def test_snap_build_preserves_optional_metadata_presence(self):
        source = (
            ROOT
            / "Goat 4"
            / "snap"
            / "goat4_snapmaker_u1_contour_packed_purge_safe_faster.3mf"
        )
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            output = tmp / "snap.3mf"
            self.module.build_project(source, self.placements_file(tmp), output)
            self.assert_common_integrity(source, output)
            with zipfile.ZipFile(source) as original, zipfile.ZipFile(output) as built:
                for name in ["Metadata/cut_information.xml", "Metadata/plate_1.json"]:
                    self.assertEqual(name in original.namelist(), name in built.namelist())

    def test_exact_verifier_reports_edge_and_purge_line_clearances(self):
        source = (
            ROOT
            / "Goat 4"
            / "snap"
            / "goat4_snapmaker_u1_contour_packed_purge_safe_faster.3mf"
        )
        stl = ROOT / "Goat 4" / "source" / "goat4_v2.stl"
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            placements = self.placements_file(tmp)
            output = tmp / "snap.3mf"
            self.module.build_project(source, placements, output)
            report = self.verify_module.verify(
                stl,
                placements,
                output,
                minimum_gap=0.5,
                edge_margin=3.5,
                purge_lines=[(85.0, 1.0, 185.0, 1.0, 0.5)],
                purge_gap=0.5,
            )
            self.assertGreaterEqual(report["minimum_edge_clearance_mm"], 3.5)
            self.assertGreaterEqual(report["minimum_purge_gap_mm"], 0.5)

    def test_colored_source_mesh_is_reused_without_touching_paint_attributes(self):
        source = ROOT / "Goat 3" / "p1s" / "goat3_fast_max.3mf"
        stl = ROOT / "Goat 3" / "source" / "goat3.stl"
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            placements = self.placements_file(tmp)
            output = tmp / "colored.3mf"
            self.module.build_project(source, placements, output, stl)
            with zipfile.ZipFile(source) as original, zipfile.ZipFile(output) as built:
                core_ns = {"c": "http://schemas.microsoft.com/3dmanufacturing/core/2015/02"}
                source_root = ET.fromstring(original.read("3D/3dmodel.model"))
                output_root = ET.fromstring(built.read("3D/3dmodel.model"))
                source_component = source_root.find(".//c:component", core_ns)
                output_components = output_root.findall(".//c:component", core_ns)
                source_path = source_component.get("{http://schemas.microsoft.com/3dmanufacturing/production/2015/06}path").lstrip("/")
                self.assertEqual(len(output_components), 2)
                self.assertTrue(all(c.get("{http://schemas.microsoft.com/3dmanufacturing/production/2015/06}path").lstrip("/") == source_path for c in output_components))
                self.assertEqual(original.read(source_path), built.read(source_path))
                self.assertIn(b"paint_color", built.read(source_path))

    def test_preview_accepts_custom_keepout_with_y_axis_flip(self):
        source = (
            ROOT
            / "Goat 3"
            / "p1s"
            / "goat3_fast_max.3mf"
        )
        stl = ROOT / "Goat 3" / "source" / "goat3.stl"
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            output = tmp / "keepout_preview.3mf"
            self.module.build_project(
                source,
                self.placements_file(tmp),
                output,
                stl,
                custom_keepouts=[(226.85, 212.35, 245.15, 248.65)],
            )
            with zipfile.ZipFile(output) as built:
                self.assertEqual(Image.open(io.BytesIO(built.read("Metadata/plate_1.png"))).size, (512, 512))

    def test_colored_clone_can_duplicate_mesh_part_per_object(self):
        source = ROOT / "Goat 3" / "snap" / "goat3_snap.3mf"
        stl = ROOT / "Goat 3" / "source" / "goat3.stl"
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            output = tmp / "duplicate_mesh.3mf"
            self.module.build_project(
                source,
                self.placements_file(tmp),
                output,
                stl,
                duplicate_mesh_per_object=True,
            )
            core_ns = {"c": "http://schemas.microsoft.com/3dmanufacturing/core/2015/02"}
            with zipfile.ZipFile(source) as original, zipfile.ZipFile(output) as built:
                root = ET.fromstring(built.read("3D/3dmodel.model"))
                paths = [
                    c.get("{http://schemas.microsoft.com/3dmanufacturing/production/2015/06}path").lstrip("/")
                    for c in root.findall(".//c:component", core_ns)
                ]
                self.assertEqual(len(paths), 2)
                self.assertEqual(len(set(paths)), 2)
                original_path = next(iter(paths))
                self.assertIn(b"paint_color", built.read(original_path))
                self.assertEqual(built.read(paths[0]), built.read(paths[1]))

    def test_colored_clone_can_use_standard_3mf_instances(self):
        source = ROOT / "Goat 3" / "snap" / "goat3_snap.3mf"
        stl = ROOT / "Goat 3" / "source" / "goat3.stl"
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            output = tmp / "instances.3mf"
            self.module.build_project(
                source,
                self.placements_file(tmp),
                output,
                stl,
                reuse_source_object_instances=True,
            )
            core_ns = {"c": "http://schemas.microsoft.com/3dmanufacturing/core/2015/02"}
            with zipfile.ZipFile(output) as built:
                root = ET.fromstring(built.read("3D/3dmodel.model"))
                resources = root.findall(".//c:resources/c:object", core_ns)
                items = root.findall(".//c:build/c:item", core_ns)
                self.assertEqual(len(resources), 1)
                self.assertEqual([item.get("objectid") for item in items], ["2", "2"])
                settings = ET.fromstring(built.read("Metadata/model_settings.config"))
                self.assertEqual([item.get("id") for item in settings.findall("object")], ["2"])
                instances = settings.findall("plate/model_instance")
                self.assertEqual(len(instances), 1, "preserve Orca-compatible source metadata")
            report = self.verify_module.verify(
                stl,
                self.placements_file(tmp),
                output,
                minimum_gap=0.0,
                edge_margin=0.0,
            )
            self.assertEqual(report["count"], 2)

    def test_colored_clone_can_inline_painted_meshes_for_slicer_compatibility(self):
        source = ROOT / "Goat 3" / "snap" / "goat3_snap.3mf"
        stl = ROOT / "Goat 3" / "source" / "goat3.stl"
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            output = tmp / "inline_meshes.3mf"
            self.module.build_project(
                source,
                self.placements_file(tmp),
                output,
                stl,
                inline_colored_mesh_objects=True,
            )
            core_ns = {"c": "http://schemas.microsoft.com/3dmanufacturing/core/2015/02"}
            with zipfile.ZipFile(output) as built:
                root = ET.fromstring(built.read("3D/3dmodel.model"))
                resources = root.findall(".//c:resources/c:object", core_ns)
                self.assertEqual(len(resources), 2)
                self.assertTrue(all(obj.find("c:mesh", core_ns) is not None for obj in resources))
                self.assertEqual(len(root.findall(".//c:component", core_ns)), 0)
                self.assertIn(b"paint_color", built.read("3D/3dmodel.model"))

    def test_colored_clone_preserves_source_z_offset(self):
        source = ROOT / "Goat 3" / "snap" / "goat3_snap.3mf"
        stl = ROOT / "Goat 3" / "source" / "goat3.stl"
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            output = tmp / "source_z.3mf"
            self.module.build_project(
                source,
                self.placements_file(tmp),
                output,
                stl,
                reuse_source_object_instances=True,
            )
            core_ns = {"c": "http://schemas.microsoft.com/3dmanufacturing/core/2015/02"}
            with zipfile.ZipFile(output) as built:
                root = ET.fromstring(built.read("3D/3dmodel.model"))
                z_values = [
                    float(item.get("transform").split()[11])
                    for item in root.findall(".//c:build/c:item", core_ns)
                ]
                self.assertEqual(z_values, [4.30000019, 4.30000019])

    def synthetic_source_3mf(self, directory: Path) -> Path:
        """A self-contained single-object 3MF whose component carries its own translation.

        This is the structure that broke the real Goat 3 P1S pack: mesh centered on the local
        origin, component transform translating it by (10, 20, -0.5). Without compensation every
        clone lands offset from its packed placement and can fall off the bed.
        """
        mesh_part = (
            '<?xml version="1.0" encoding="UTF-8"?>'
            '<model unit="millimeter" xml:lang="en-US"'
            ' xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02">'
            "<resources><object id=\"1\" type=\"model\"><mesh><vertices>"
            '<vertex x="0" y="0" z="0"/>'
            '<vertex x="10" y="0" z="0"/>'
            '<vertex x="10" y="10" z="0"/>'
            '<vertex x="0" y="10" z="0"/>'
            "</vertices><triangles>"
            '<triangle v1="0" v2="1" v3="2"/>'
            '<triangle v1="0" v2="2" v3="3"/>'
            "</triangles></mesh></object></resources></model>"
        )
        core = (
            '<?xml version="1.0" encoding="UTF-8"?>'
            '<model unit="millimeter" xml:lang="en-US"'
            ' xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02"'
            ' xmlns:p="http://schemas.microsoft.com/3dmanufacturing/production/2015/06">'
            '<resources><object id="2" type="model"><components>'
            '<component p:path="/3D/Objects/mesh.model" transform="1 0 0 0 1 0 0 0 1 10 20 -0.5"/>'
            "</components></object></resources>"
            '<build><item objectid="2" transform="1 0 0 0 1 0 0 0 1 50 60 0.5"/></build>'
            "</model>"
        )
        settings = json.dumps(
            {
                "printer_model": "Synthetic Printer",
                "printable_area": ["0x0", "200x0", "200x200", "0x200"],
            }
        )
        model_settings = '<config><object id="2"/><plate/></config>'
        source = directory / "synthetic_source.3mf"
        with zipfile.ZipFile(source, "w") as archive:
            archive.writestr("3D/3dmodel.model", core)
            archive.writestr("3D/Objects/mesh.model", mesh_part)
            archive.writestr("Metadata/project_settings.config", settings)
            archive.writestr("Metadata/model_settings.config", model_settings)
        return source

    def test_extracted_stl_bakes_component_transform(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            source = self.synthetic_source_3mf(tmp)
            triangles = self.extract_module.embedded_mesh_triangles(source)
            corners = {tuple(round(value, 6) for value in corner) for triangle in triangles for corner in triangle}
            self.assertEqual(
                corners,
                {(10.0, 20.0, -0.5), (20.0, 20.0, -0.5), (20.0, 30.0, -0.5), (10.0, 30.0, -0.5)},
            )

    def test_item_transform_compensates_component_translation(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            source = self.synthetic_source_3mf(tmp)
            stl = tmp / "embedded_baked.stl"
            self.extract_module.write_binary_stl(
                stl, self.extract_module.embedded_mesh_triangles(source)
            )
            output = tmp / "synthetic_packed.3mf"
            self.module.build_project(source, self.placements_file(tmp), output, stl)
            with zipfile.ZipFile(output) as built:
                root = ET.fromstring(built.read("3D/3dmodel.model"))
                items = root.findall(".//c:build/c:item", CORE_NS)
                self.assertEqual(len(items), 2)
                # (x, y) refers to the baked mesh's bbox center (15, 25): the item translation
                # must rotate that center out, not use the raw placement.
                self.assert_transform_matches(
                    items[0].get("transform"),
                    self.expected_item_transform({"x_mm": 60.0, "y_mm": 70.0, "angle_deg": 0.0}, (15.0, 25.0), 0.5),
                )
                self.assert_transform_matches(
                    items[1].get("transform"),
                    self.expected_item_transform({"x_mm": 150.0, "y_mm": 160.0, "angle_deg": 90.0}, (15.0, 25.0), 0.5),
                )
                # Independent end-to-end check: each clone's world-space bbox center (embedded
                # mesh through the kept component transform, then the item transform) must equal
                # its packed placement exactly.
                component = root.find(".//c:resources/c:object/c:components/c:component", CORE_NS)
                component_matrix = self.extract_module.parse_transform(component.get("transform"))
                mesh_root = ET.fromstring(built.read("3D/Objects/mesh.model"))
                embedded = [
                    (float(v.get("x")), float(v.get("y")), float(v.get("z")))
                    for v in mesh_root.findall(".//c:vertex", CORE_NS)
                ]
                for item, expected in zip(items, [(60.0, 70.0), (150.0, 160.0)]):
                    item_matrix = self.extract_module.parse_transform(item.get("transform"))

                    def through(point):
                        local = self.extract_module.apply_matrix(component_matrix, point)
                        return self.extract_module.apply_matrix(item_matrix, local)

                    world = [through(point) for point in embedded]
                    center = (
                        sum(p[0] for p in world) / len(world),
                        sum(p[1] for p in world) / len(world),
                    )
                    self.assertAlmostEqual(center[0], expected[0], places=6)
                    self.assertAlmostEqual(center[1], expected[1], places=6)
            report = self.verify_module.verify(
                stl,
                self.placements_file(tmp),
                output,
                minimum_gap=0.0,
                edge_margin=0.0,
            )
            self.assertEqual(report["count"], 2)


if __name__ == "__main__":
    unittest.main()
