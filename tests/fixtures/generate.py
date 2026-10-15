"""Regenerate tiny, deterministic synthetic 3MF integration fixtures (Python stdlib only).
Not run by tests. Real geometry regression assets are intentionally NOT regenerated here.
"""
from pathlib import Path
import json
import struct
import zipfile

ROOT = Path(__file__).resolve().parent
NS = 'xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02" xmlns:p="http://schemas.microsoft.com/3dmanufacturing/production/2015/06"'
MESH = f'''<model unit="millimeter" {NS}><resources><object id="1" type="model"><mesh>
<vertices><vertex x="0" y="0" z="0"/><vertex x="10" y="0" z="0"/><vertex x="10" y="10" z="0"/><vertex x="0" y="10" z="0"/></vertices>
<triangles><triangle v1="0" v2="1" v3="2" paint_color="4"/><triangle v1="0" v2="2" v3="3" paint_color="8"/></triangles>
</mesh></object></resources></model>'''

def write_project(name, count=1, plates=1, z=3.70000005, bambu=True):
    objects, items, configs, instances, assembled = [], [], [], [], []
    for i in range(count):
        id = i + 2
        x, y = 20 + i % 6 * 30, 40 + i // 6 * 30
        objects.append(f'<object id="{id}" type="model" p:UUID="object-{id}"><components><component objectid="1" p:path="/3D/Objects/mesh.model" transform="1 0 0 0 1 0 0 0 1 10 20 -{z}"/></components></object>')
        items.append(f'<item objectid="{id}" p:UUID="item-{id}" transform="1 0 0 0 1 0 0 0 1 {x} {y} {z}" printable="1"/>')
        configs.append(f'<object id="{id}"><metadata key="name" value="Fixture {id}"/><part id="1" subtype="normal_part"><metadata key="extruder" value="1"/></part></object>')
        instances.append(f'<model_instance><metadata key="object_id" value="{id}"/><metadata key="instance_id" value="0"/><metadata key="identify_id" value="{id}0"/></model_instance>')
        assembled.append(f'<assemble_item object_id="{id}" instance_id="0" transform="1 0 0 0 1 0 0 0 1 0 0 {z}" offset="0 0 0"/>')
    plate_xml = ''.join(f'<plate><metadata key="plater_id" value="{p+1}"/><metadata key="plater_name" value="Fixture {p+1}"/>{"".join(instances[p::plates])}</plate>' for p in range(plates))
    entries = {
        '[Content_Types].xml': '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/></Types>',
        '_rels/.rels': '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Target="/3D/3dmodel.model" Id="root" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/></Relationships>',
        '3D/3dmodel.model': f'<model unit="millimeter" xml:lang="en-US" {NS}><metadata name="Application">BambuStudio-02.00.00.00</metadata><resources>{"".join(objects)}</resources><build>{"".join(items)}</build></model>',
        '3D/Objects/mesh.model': MESH,
        '3D/_rels/3dmodel.model.rels': '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Target="/3D/Objects/mesh.model" Id="mesh" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/></Relationships>',
        'Metadata/model_settings.config': f'<config>{"".join(configs)}{plate_xml}<assemble>{"".join(assembled)}</assemble></config>',
        'Metadata/project_settings.config': json.dumps({'printer_model': 'Bambu Lab P1S' if bambu else 'Snapmaker U1', 'printable_area': ['0x0','256x0','256x256','0x256'], 'bed_exclude_area': [], 'machine_start_gcode': 'G28 ; fixture', 'wipe_tower_x': ['230.962'], 'wipe_tower_y': ['160']}, sort_keys=True),
    }
    if bambu:
        entries['Metadata/cut_information.xml'] = '<objects><object id="1"/></objects>'
        entries['Metadata/plate_1.json'] = json.dumps({'bbox_objects': [], 'bbox_all': [0, 0, 10, 10]})
    with zipfile.ZipFile(ROOT / name, 'w') as archive:
        for path, content in sorted(entries.items()):
            entry = zipfile.ZipInfo(path, (2026, 1, 1, 0, 0, 0))
            entry.compress_type = zipfile.ZIP_DEFLATED
            archive.writestr(entry, content)

write_project('bambu.3mf')
write_project('snap.3mf', bambu=False)
write_project('colored_bambu.3mf', z=4.30000019)
write_project('colored_snap.3mf', z=4.30000019, bambu=False)
write_project('packed.3mf', count=29)
write_project('packed_snap.3mf', count=29, bambu=False)
write_project('three_plates.3mf', count=9, plates=3)
# Collision STL has the component's XY translation baked in, matching embedded_mesh.
triangles = [((10.,20.,0.),(20.,20.,0.),(20.,30.,0.)), ((10.,20.,0.),(20.,30.,0.),(10.,30.,0.))]
with (ROOT / 'square.stl').open('wb') as f:
    f.write(bytes(80) + struct.pack('<I', len(triangles)))
    for triangle in triangles:
        f.write(struct.pack('<12fH', 0,0,0, *(v for p in triangle for v in p), 0))
