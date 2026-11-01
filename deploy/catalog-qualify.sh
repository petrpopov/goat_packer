#!/usr/bin/env bash
# Квалификация черновиков каталога принтеров (`printer_catalog extract`) в runner-образах на этом хосте.
#   deploy/catalog-qualify.sh [-t v0.3.0] [-P 2] [-B <каталог Linux-бинарников>] <черновики> <orca/resources/profiles> <выход>
# На выходе: <выход>/profiles/<id>.json (только прошедшие), <выход>/report.md (строка на каждый принтер),
# <выход>/work/<id>/ (пресеты, probe.3mf, логи слайсера/пакера/verify).
# Образ по slicer_kind черновика: goat-packer-runner-orca:<тег> / goat-packer-runner-bambu:<тег>; тег по умолчанию —
# последний v* из локального Docker. probe.3mf всегда экспортирует Orca CLI (runner-orca), поэтому принтер Bambu
# проходит в два запуска: --stage export в runner-orca, --stage pack в runner-bambu (там /opt/slicers/*/AppRun = BambuStudio).
# Бинарники берутся из образа (/usr/local/bin); -B подменяет все три (printer_catalog, colored_3mf_packer,
# verify_outputs) сборкой из каталога — нужно, пока образ старше printer_catalog или фикса verify_outputs.
# Сеть контейнеру не нужна: --network none, как у задач сервиса.
set -euo pipefail

tag=""
jobs=2
bin_dir=""
while getopts "t:P:B:" opt; do
  case $opt in
    t) tag=$OPTARG ;;
    P) jobs=$OPTARG ;;
    B) bin_dir=$(cd "$OPTARG" && pwd) ;;
    *) echo "usage: $0 [-t tag] [-P jobs] [-B linux-bin-dir] <drafts-dir> <orca-profiles-dir> <out-dir>" >&2; exit 2 ;;
  esac
done
shift $((OPTIND - 1))
[ $# -eq 3 ] || { echo "usage: $0 [-t tag] [-P jobs] [-B linux-bin-dir] <drafts-dir> <orca-profiles-dir> <out-dir>" >&2; exit 2; }
drafts=$(cd "$1" && pwd)
orca=$(cd "$2" && pwd)
mkdir -p "$3"
out=$(cd "$3" && pwd)

# Последний релизный тег, для которого локально есть оба runner-образа.
if [ -z "$tag" ]; then
  tag=$(comm -12 \
    <(docker image ls goat-packer-runner-orca --format '{{.Tag}}' | grep '^v' | sort -u) \
    <(docker image ls goat-packer-runner-bambu --format '{{.Tag}}' | grep '^v' | sort -u) | sort -V | tail -n 1)
  [ -n "$tag" ] || { echo "нет локальных goat-packer-runner-{orca,bambu}:v*; укажите -t" >&2; exit 1; }
fi
for kind in orca bambu; do
  docker image inspect "goat-packer-runner-$kind:$tag" >/dev/null 2>&1 || { echo "нет образа goat-packer-runner-$kind:$tag" >&2; exit 1; }
done
if [ -n "$bin_dir" ]; then
  for b in printer_catalog colored_3mf_packer verify_outputs; do
    [ -x "$bin_dir/$b" ] || { echo "$bin_dir/$b: нет исполняемого файла (нужна Linux x86-64 сборка)" >&2; exit 1; }
  done
fi

mkdir -p "$out/profiles" "$out/work"
# Шапку пишем до параллельных запусков: иначе два первых контейнера могут оба решить, что файл новый.
[ -f "$out/report.md" ] || printf '| vendor | printer_model | id | result | reason | seconds |\n|---|---|---|---|---|---|\n' > "$out/report.md"

export tag orca drafts out bin_dir
export docker_user=${DOCKER_USER:-$(id -u):$(id -g)}

# Один черновик: export (+ pack) в нужных образах. Код 3 = не прошёл, строка в отчёте уже есть;
# любой другой ненулевой код (паника, timeout, OOM-kill) — строку пишем сами.
qualify_one() {
  local file=$1 id kind rc
  id=$(basename "$file" .json)
  kind=$(sed -n 's/^ *"slicer_kind": *"\([a-z]*\)".*/\1/p' "$drafts/$file" | head -n 1)
  [ -n "$kind" ] || kind=orca
  local work=/out/work/$id
  mkdir -p "$out/work/$id/home"
  local bin=/usr/local/bin mounts=(-v "$drafts:/drafts:ro" -v "$orca:/orca:ro" -v "$out:/out")
  if [ -n "$bin_dir" ]; then bin=/catalog-bin; mounts+=(-v "$bin_dir:/catalog-bin:ro"); fi
  run() { # <kind> <stage>
    # timeout внутри контейнера: зависший слайсер не держит слот бесконечно (пакер делает несколько срезов).
    # -w "$work": Orca CLI пишет свой лог 00000.log в текущий каталог; WORKDIR образа (/work) не записываем
    # для не-root пользователя, и без -w экспорт падал у всех принтеров (boost filesystem_error, exit 134).
    docker run --rm --init --network none --cpus 2 --memory 4g --user "$docker_user" \
      -w "$work" -e HOME="$work/home" "${mounts[@]}" "goat-packer-runner-$1:$tag" \
      timeout 2400 "$bin/printer_catalog" qualify --draft "/drafts/$file" --orca /orca \
      --packer "$bin/colored_3mf_packer" --verify "$bin/verify_outputs" \
      --work "$work" --out /out/profiles --report /out/report.md --stage "$2" \
      > "$out/work/$id/$2.out" 2>&1
  }
  rc=0
  if [ "$kind" = bambu ]; then
    run orca export || rc=$?
    [ "$rc" -ne 0 ] || run bambu pack || rc=$?
  else
    run orca all || rc=$?
  fi
  if [ "$rc" -ne 0 ] && [ "$rc" -ne 3 ]; then
    printf '| ? | ? | %s | failed | wrapper: exit %s (124 = timeout), см. work/%s/*.out | ? |\n' "$id" "$rc" "$id" >> "$out/report.md"
  fi
  echo "$id: $([ "$rc" -eq 0 ] && echo qualified || echo "failed ($rc)")"
}
export -f qualify_one

# $1 раскрывает дочерний bash (имя черновика от xargs), а не этот: одинарные кавычки намеренно.
# shellcheck disable=SC2016
find "$drafts" -maxdepth 1 -name '*.json' -exec basename {} \; | sort | xargs -P "$jobs" -I{} bash -c 'set -euo pipefail; qualify_one "$1"' _ {}

echo "отчёт: $out/report.md; прошли: $(grep -c '| qualified |' "$out/report.md" || true), не прошли: $(grep -c '| failed |' "$out/report.md" || true)"
