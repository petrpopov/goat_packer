#!/bin/bash
# Фейковый ams_calibration для интеграционных тестов: пишет аргументы построчно в
# calibration-args.txt (рабочий каталог джоба), затем в --out-dir — calib_A.3mf, calib_B.3mf,
# instructions.md, calibration.json и черновик slicing/ (его наружу отдавать нельзя).
# Шаблон с одним филаментом ("filament_colour":["…"] в несжатом project_settings.config)
# воспроизводит настоящую ошибку бинарника. Слайсер не вызывается.
set -euo pipefail

printf '%s\n' "$@" > calibration-args.txt

template=""
variant=""
out_dir=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --template) template="$2"; shift 2;;
    --variant) variant="$2"; shift 2;;
    --out-dir) out_dir="$2"; shift 2;;
    *) shift 2;;
  esac
done

if grep -aEq '"filament_colour":\["[^"]*"\]' "$template"; then
  echo "error: для теста нужен проект минимум с двумя филаментами" >&2
  exit 1
fi

changes=10
[[ "$variant" == "perfectionist" ]] && changes=30

mkdir -p "$out_dir/slicing"
printf 'fake calib A\n' > "$out_dir/calib_A.3mf"
printf 'fake calib B\n' > "$out_dir/calib_B.3mf"
printf 'scratch\n' > "$out_dir/slicing/calib_A_slice.log"
printf '# Калибровка времени смены филамента\n\nВариант: **%s** — %s смен.\n\n- `calib_A.3mf`\n- `calib_B.3mf`\n' "$variant" "$changes" > "$out_dir/instructions.md"
report='{
  "variant": "'"$variant"'",
  "changes_expected": '"$changes"',
  "changes_sliced": '"$changes"',
  "estimate_a_s": 1500.0,
  "estimate_b_s": 2400.0,
  "slicer_change_s": 20.5
}'
printf '%s\n' "$report" > "$out_dir/calibration.json"
printf '%s\n' "$report"
