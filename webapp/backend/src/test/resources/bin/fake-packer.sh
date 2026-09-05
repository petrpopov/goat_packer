#!/usr/bin/env bash
# Фейковый colored_3mf_packer для интеграционных тестов: печатает характерные строки прогресса
# (ровно те, что разбирает ProgressTracker) и создаёт артефакты по --output: packed.3mf и
# packed.verification.json с object_count. Содержимое 3MF — фикция, слайсер не вызывается.
set -euo pipefail

output="packed.3mf"
profile=""
source_file=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --source) source_file="$2"; shift 2;;
    --profile) profile="$2"; shift 2;;
    --output) output="$2"; shift 2;;
    *) shift;;
  esac
done

verification="${output%.3mf}.verification.json"

echo "probing prime tower position (slicing 1 object)..."
echo "probe tower keepout: 10.00,10.00 → 30.00,30.00"
echo "packing (2000 attempts, seed 20270831)..."
echo "pack_footprint: 240/2000 attempts, best so far: 12 objects"
echo "pack_footprint: 2000/2000 attempts, best so far: 30 objects"
echo "pack_footprint: best layout: 30 objects (found at attempt 1800, 2000/2000 attempts run)"
echo "packed 30 objects; assembling 3MF..."
echo "slicing final plate (30 objects)..."

mkdir -p "$(dirname "$output")"
printf 'fake packed 3mf content for tests\n' > "$output"
printf '{"object_count": 30, "output_3mf": "%s", "profile": "%s"}\n' "$output" "$profile" > "$verification"

echo "packed 30 colored objects; wrote $output"
echo "report: $verification"
