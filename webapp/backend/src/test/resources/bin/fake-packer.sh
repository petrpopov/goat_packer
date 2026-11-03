#!/bin/bash
# Фейковый colored_3mf_packer для интеграционных тестов: печатает характерные строки прогресса
# (ровно те, что разбирает ProgressTracker) и создаёт артефакты по --output: packed.3mf и
# packed.verification.json с object_count и блоком time. Содержимое 3MF — фикция, слайсер не
# вызывается. Аргументы пишутся построчно в packer-args.txt (рабочий каталог джоба), чтобы
# тесты проверяли сборку CLI; --time-window-hours W даёт стол «12 из 30» с окном W;
# --support-overlap-mm X добавляет в отчёт supports_detected/support_overlap_mm.
set -euo pipefail

printf '%s\n' "$@" > packer-args.txt

output="packed.3mf"
profile=""
source_file=""
window_hours=""
support_overlap=""
verify=false
finish_state=""
cache_dir=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --source) source_file="$2"; shift 2;;
    --profile) profile="$2"; shift 2;;
    --output) output="$2"; shift 2;;
    --time-window-hours) window_hours="$2"; shift 2;;
    --support-overlap-mm) support_overlap="$2"; shift 2;;
    --cache-dir) cache_dir="$2"; shift 2;;
    --verify) verify=true; shift;;
    --finish-state) finish_state="$2"; verify=true; shift 2;;
    *) shift;;
  esac
done

# Окно времени нарезает пластины всегда.
if [[ -n "$window_hours" ]]; then verify=true; fi

verification="${output%.3mf}.verification.json"

# --cache-dir: запись пробы под ключом от входа (как пакер — по точному ключу): есть — «cache hit», нет — «проба»
# и запись. Подделку (неверный ключ внутри) пишем всегда: воркер обязан её отбросить. Итог — в cache-result.txt.
if [[ -n "$cache_dir" ]]; then
  # Как у пакера, ключ — от сетки, а не от файла: вершины и треугольники проекта (нет сетки — весь файл).
  # CI's JDK image has no unzip: fall back to the JDK's jar.
  model_xml() {
    if command -v unzip >/dev/null; then unzip -p "$source_file" 3D/3dmodel.model 2>/dev/null; return; fi
    local abs tmp; abs="$(cd "$(dirname "$source_file")" && pwd)/$(basename "$source_file")"; tmp="$(mktemp -d)"
    (cd "$tmp" && jar xf "$abs" 3D/3dmodel.model 2>/dev/null && cat 3D/3dmodel.model); rm -rf "$tmp"
  }
  mesh="$(model_xml | grep -oE '<(vertex|triangle) [^>]*>' || true)"
  if [[ -n "$mesh" ]]; then
    key="$(printf '%s' "$mesh" | (shasum -a 256 2>/dev/null || sha256sum) | cut -c1-64)"
  else
    key="$( (shasum -a 256 "$source_file" 2>/dev/null || sha256sum "$source_file") | cut -c1-64)"
  fi
  entry="$cache_dir/probe-$key.json"
  mkdir -p "$cache_dir"
  if [[ -f "$entry" ]]; then
    echo "support probe of rotation 0°: cache hit (3 runs)"; echo hit > cache-result.txt
  else
    echo "support probe of rotation 0°: slicing"; echo miss > cache-result.txt
    printf '{"rotation_deg":0.0,"runs":3,"cache_key":"%s"}\n' "$key" > "$entry"
  fi
  printf '{"x":1}\n' > "$cache_dir/support-$(printf 'b%.0s' $(seq 64)).json"
fi

echo "probing prime tower position (slicing 1 object)..."
echo "probe tower keepout: 10.00,10.00 → 30.00,30.00"
echo "packing (2000 attempts, seed 20270831)..."
echo "pack_footprint: 240/2000 attempts, best so far: 12 objects"
echo "pack_footprint: 2000/2000 attempts, best so far: 30 objects"
echo "pack_footprint: best layout: 30 objects (found at attempt 1800, 2000/2000 attempts run)"
# layout-событие (как у настоящего пакера) — только при GOAT_LOG_FORMAT=json, в stderr.
if [[ "${GOAT_LOG_FORMAT:-}" == "json" ]]; then
  echo '{"event":"layout","source":"pack","objects":2,"bed":[0,0,256,256],"keepouts":[],"tower":null,"footprint":[[[-5,-5],[5,-5],[5,5],[-5,5],[-5,-5]]],"placements":[[20,20,0],[40,20,90]]}' >&2
fi
echo "packed 30 objects; assembling 3MF..."
if [[ "$verify" == "true" ]]; then echo "slicing final plate (30 objects)..."; fi

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
# С --support-overlap-mm отчёт несёт поля поддержек; без него — нет (как у старого пакера).
supports_block=""
if [[ -n "$support_overlap" ]]; then
  supports_block=', "supports_detected": true, "support_overlap_mm": '"$support_overlap"
fi
# --finish-state: состояние родителя обязано лежать рядом (его кладёт бэкенд).
if [[ -n "$finish_state" && ! -f "$finish_state" ]]; then echo "error: no state $finish_state" >&2; exit 1; fi
# Без --verify: стол без нарезки — состояние рядом с --output, времени нет.
if [[ "$verify" != "true" ]]; then
  printf '{"fake": "state"}\n' > "${output%.3mf}.state.json"
  time_block=null
  supports_block="$supports_block"', "final_sliced": false'
fi
# Башня очистки (как в настоящем отчёте: угол wipe_tower_x/y и размер), совпадает с keepout пробы.
tower_block='{"anchor": {"x": 10.0, "y": 10.0}, "size": [20.0, 20.0], "brim_mm": 3.0, "chamfer_rings": 0, "source": "probe+model"}'
printf '{"object_count": %s, "output_3mf": "%s", "profile": "%s", "time": %s, "tower": %s%s}\n' "$count" "$output" "$profile" "$time_block" "$tower_block" "$supports_block" > "$verification"

echo "packed 30 colored objects; wrote $output"
mkdir -p work
printf '{"placements": [], "cache": "%s"}\n' "$(cat cache-result.txt 2>/dev/null || echo none)" > work/placements.json
echo "report: $verification"
