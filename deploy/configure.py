#!/usr/bin/env python3
"""Size deployment against the Docker host. Existing secrets require explicit --update and are preserved."""
import argparse
import ipaddress
import json
import math
import os
from pathlib import Path
import secrets
import subprocess


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--memory-mib', type=int, help='RAM budget allocated to this service')
    parser.add_argument('--cpus', type=float, help='CPU budget allocated to this service')
    parser.add_argument('--storage-gib', type=int, required=True)
    parser.add_argument('--public-ip', help='IPv4 address reachable by users; otherwise localhost only')
    parser.add_argument('--update', action='store_true', help='update resource limits, preserving existing credentials and ports')
    args = parser.parse_args()
    path = Path(__file__).resolve().parent / '.env'
    if path.exists() and not args.update:
        raise SystemExit('Config exists. Use --update to resize while preserving secrets, or edit deploy/.env.')
    existing = {}
    if path.exists():
        existing = dict(line.split('=', 1) for line in path.read_text().splitlines() if line and not line.startswith('#') and '=' in line)
    if args.public_ip:
        ipaddress.IPv4Address(args.public_ip)
    if args.storage_gib < 1 or (args.cpus is not None and (not math.isfinite(args.cpus) or args.cpus <= 0)) or (args.memory_mib is not None and args.memory_mib <= 0):
        raise SystemExit('Budgets must be positive finite values')
    running = subprocess.check_output(['docker', 'ps', '-q']).strip()
    if running and (args.memory_mib is None or args.cpus is None):
        raise SystemExit('Other containers run here: specify --cpus and --memory-mib allocated to this service.')
    info = json.loads(subprocess.check_output(['docker', 'info', '--format', '{{json .}}']))
    ram = min(args.memory_mib or info['MemTotal']//1048576, info['MemTotal']//1048576)
    cpus = min(args.cpus or info['NCPU'], info['NCPU'])
    # Host 768 + API768 + PG384 + S3 512 + worker512 + proxy64 + safety320.
    reserve = 3328
    job_memory, job_cpus = 4096, 2
    slots = max(0, min(8, math.floor((ram-reserve)/job_memory), math.floor((cpus-1)/job_cpus)))
    values = {
        'DATABASE_PASSWORD': secrets.token_urlsafe(32), 'S3_SECRET_KEY': secrets.token_urlsafe(32),
        'ADMIN_LOGIN': 'admin', 'ADMIN_PASSWORD': secrets.token_urlsafe(24),
        'COOKIE_SECURE': 'false', 'WEB_PORT': '8088', 'OBJECT_PORT': '8091',
        'BIND_ADDRESS': '127.0.0.1', 'S3_PUBLIC_ENDPOINT': 'http://localhost:8091',
        'SCRATCH_ROOT': '/srv/goat/scratch',
    }
    values.update(existing)
    values.update({
        'MAX_PARALLEL': str(slots), 'EXECUTION_CPU_BUDGET': str(slots*job_cpus),
        'EXECUTION_MEMORY_MIB': str(slots*job_memory), 'WORKER_CPUS': str(job_cpus),
        'WORKER_MEMORY_MIB': str(job_memory), 'STORAGE_LIMIT_BYTES': str(args.storage_gib*1024**3),
        'PRODUCTION_MODE': 'true', 'REQUIRE_CHALLENGE': 'true',
    })
    for kind, version in [('BAMBU', '2.8.2.61'), ('ORCA', '2.4.2')]:
        key = f'RUNNER_{kind}_IMAGE'
        if not values.get(key): values[key] = f'goat-runner-{kind.lower()}:{version}'
    if args.public_ip:
        values['BIND_ADDRESS'] = '0.0.0.0'
        values['S3_PUBLIC_ENDPOINT'] = f'http://{args.public_ip}:{values["OBJECT_PORT"]}'
    temporary = path.with_suffix('.env-new')
    fd = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, 'w') as f: f.write('\n'.join(f'{k}={v}' for k, v in values.items())+'\n')
    temporary.replace(path)
    print(f'Config saved (0600); CPU={cpus}, RAM={ram} MiB, native slots={slots}. Secrets are not printed.')
    if not slots: print('No safe execution slots: increase the resource budget. Queue will remain paused.')
    if values['COOKIE_SECURE'] == 'false': print('HTTP/IP mode: passwords and session cookies are not encrypted in transit.')
    print('Next: sh deploy/start.sh (downloads/builds pinned slicers once; jobs themselves run offline).')

if __name__ == '__main__': main()
