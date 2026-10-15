"""Rebuild a small closed cube with checked-in standalone slicer settings; no Downloads needed."""
from pathlib import Path
import json
import zipfile
root=Path(__file__).resolve().parent
with zipfile.ZipFile(root/'bambu.3mf') as z: entries={n:z.read(n) for n in z.namelist()}
vertices=[(0,0,0),(80,0,0),(80,80,0),(0,80,0),(0,0,0.8),(80,0,0.8),(80,80,0.8),(0,80,0.8)]
triangles=[(0,2,1),(0,3,2),(4,5,6),(4,6,7),(0,1,5),(0,5,4),(1,2,6),(1,6,5),(2,3,7),(2,7,6),(3,0,4),(3,4,7)]
mesh='<model unit="millimeter" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02"><resources><object id="1" type="model"><mesh><vertices>'
mesh+=''.join(f'<vertex x="{x}" y="{y}" z="{z}"/>' for x,y,z in vertices)
mesh+='</vertices><triangles>'+''.join(f'<triangle v1="{a}" v2="{b}" v3="{c}"/>' for a,b,c in triangles)+'</triangles></mesh></object></resources></model>'
entries['3D/Objects/mesh.model']=mesh.encode()
entries['Metadata/project_settings.config']=(root/'native-p1s-settings.json').read_bytes()
with zipfile.ZipFile(root/'native-p1s.3mf','w') as z:
 for name,data in sorted(entries.items()):
  info=zipfile.ZipInfo(name,(2026,1,1,0,0,0));info.compress_type=zipfile.ZIP_DEFLATED;z.writestr(info,data)
