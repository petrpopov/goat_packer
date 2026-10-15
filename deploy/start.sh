#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
[ -f deploy/.env ] || { echo 'Run deploy/configure.py first' >&2; exit 1; }
python3 deploy/fetch-runners.py
docker compose --env-file deploy/.env -f deploy/compose.yml --profile images build runner-bambu runner-orca
docker compose --env-file deploy/.env -f deploy/compose.yml up --build -d --wait
