#!/bin/sh
# Оставляет N последних релизов v* образов приложения (по умолчанию 3) и убирает осиротевшие слои
# goat-packer после перезаписи тегов веток (branch-*). Базы слайсеров и образы веток не трогает.
# Образ, на котором работает контейнер, Docker не удалит — это нормально.
set -eu
keep=${1:-3}
for repo in goat-packer-api goat-packer-worker goat-packer-gateway goat-packer-runner-bambu goat-packer-runner-orca; do
  docker image ls "$repo" --format '{{.CreatedAt}}|{{.Tag}}' | grep '|v' | sort -r | awk -F'|' -v k="$keep" 'NR > k { print $2 }' |
    while read -r tag; do docker image rm "$repo:$tag" >/dev/null 2>&1 && echo "removed $repo:$tag" || true; done
done
docker image prune -f --filter label=goat-packer=1 >/dev/null
# Кэш сборок BuildKit (слои и кэши Gradle/Cargo/bun) растёт без предела: держим не больше 8 GB, старое — вон.
docker builder prune -f --max-used-space 8gb >/dev/null
