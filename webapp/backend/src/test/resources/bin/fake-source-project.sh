#!/bin/bash
# Фейковый source_project (STL → input.3mf) для интеграционных тестов: пишет аргументы построчно
# в source-args.txt, проверяет, что --stl существует, и кладёт фиктивный проект в --output.
# STL со словом FAIL внутри — ошибка в формате настоящего бинарника (её переводит CliErrorTranslator).
set -euo pipefail

printf '%s\n' "$@" > source-args.txt

stl=""
output=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --stl) stl="$2"; shift 2;;
    --output) output="$2"; shift 2;;
    *) shift;;
  esac
done

[[ -f "$stl" ]] || { echo "error: STL could not be converted: cannot read $stl: No such file or directory" >&2; exit 1; }
if grep -q FAIL "$stl"; then
  echo "error: STL could not be converted: the model's footprint 300.0x300.0 mm does not fit the printable area 250.0x210.0 mm" >&2
  exit 1
fi
printf 'fake source project\n' > "$output"
echo "source project $output: machine fake, process fake, filament fake"
