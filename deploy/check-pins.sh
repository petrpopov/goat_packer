#!/bin/sh
# Один и тот же инструмент закреплён и в шаге CI, и в Dockerfile (тесты и релизная сборка обязаны идти на одной
# версии): Woodpecker не читает версии из файла, поэтому совпадение проверяется здесь (шаг lint в check.yaml).
#   sh deploy/check-pins.sh
set -eu
cd "$(dirname "$0")/.."
status=0
for name in rust eclipse-temurin oven/bun docker; do
  # JDK (сборка, тесты) и JRE (рантайм) одной версии — разные теги одного пина: сравниваем без -jdk/-jre.
  pins=$(grep -hoE "(^FROM|image:)[[:space:]]+$name:[^[:space:]]+" deploy/Dockerfile.* .woodpecker/*.yaml |
    awk '{ print $2 }' | sed -E 's/-(jdk|jre)-/-/' | sort -u)
  if [ "$(printf '%s\n' "$pins" | grep -c .)" -gt 1 ]; then
    echo "разные версии $name: $(printf '%s' "$pins" | tr '\n' ' ')— выровняйте deploy/Dockerfile.* и .woodpecker/*.yaml" >&2
    status=1
  fi
done
[ "$status" -ne 0 ] || echo "pins: ok"
exit "$status"
