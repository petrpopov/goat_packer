#!/bin/bash
# Фейковый colored_3mf_packer для интеграционных тестов: печатает характерные строки прогресса
# (ровно те, что разбирает ProgressTracker) и создаёт артефакты по --output: packed.3mf и
# packed.verification.json с object_count и блоком time. Содержимое 3MF — фикция, слайсер не
# вызывается. Аргументы пишутся построчно в packer-args.txt (рабочий каталог джоба), чтобы
# тесты проверяли сборку CLI; --time-window-hours W даёт стол «12 из 30» с окном W.
set -euo pipefail

printf '%s\n' "$@" > packer-args.txt

output="packed.3mf"
profile=""
source_file=""
window_hours=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --source) source_file="$2"; shift 2;;
    --profile) profile="$2"; shift 2;;
    --output) output="$2"; shift 2;;
    --time-window-hours) window_hours="$2"; shift 2;;
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
if [[ -n "$window_hours" ]]; then
  # Проба (1), полный стол (30), затем попытки окна; записан префикс из 12.
  count=12
  time_block='{"window_hours": '"$window_hours"', "margin_pct": 10, "real_change_s": 95, "slicer_change_s": 20, "prep_s": 0, "samples": [{"count": 1, "estimate_s": 3600, "changes": 10, "corrected_s": 4785}, {"count": 30, "estimate_s": 72000, "changes": 300, "corrected_s": 103950}, {"count": 12, "estimate_s": 28800, "changes": 120, "corrected_s": 41580}], "chosen_count": 12, "corrected_s": 41580}'
else
  count=30
  time_block='{"window_hours": null, "margin_pct": 10, "real_change_s": null, "slicer_change_s": 20, "prep_s": 0, "samples": [{"count": 30, "estimate_s": 72000, "changes": 300, "corrected_s": 79200}], "chosen_count": 30, "corrected_s": 79200}'
fi
printf '{"object_count": %s, "output_3mf": "%s", "profile": "%s", "time": %s}\n' "$count" "$output" "$profile" "$time_block" > "$verification"

echo "packed 30 colored objects; wrote $output"
mkdir -p work
printf '{"placements": []}\n' > work/placements.json
echo "report: $verification"
