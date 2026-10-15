#!/usr/bin/env python3
"""Download only the pinned upstream assets; verify publisher's SHA256 before installation."""
from pathlib import Path
import hashlib,json,urllib.request,os
root=Path(__file__).resolve().parent
lock=json.loads((root/'runners.lock.json').read_text())
(root/'vendor').mkdir(exist_ok=True)
for kind in ['bambu','orca']:
    asset=lock[kind];path=root/'vendor'/asset['file']
    def digest(p):
        h=hashlib.sha256()
        with p.open('rb') as f:
            for data in iter(lambda:f.read(1024*1024),b''):h.update(data)
        return h.hexdigest()
    if path.exists() and digest(path)==asset['sha256']:
        print(kind,'already verified');continue
    temporary=path.with_suffix('.part')
    with urllib.request.urlopen(asset['url'],timeout=120) as source,temporary.open('wb') as dest:
        while data:=source.read(1024*1024):dest.write(data)
    if digest(temporary)!=asset['sha256']:
        temporary.unlink();raise SystemExit(f'{kind}: SHA256 mismatch; refusing asset')
    temporary.replace(path);path.chmod(0o755)
    print(kind,asset['version'],'verified',flush=True)
