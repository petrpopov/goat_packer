#!/bin/sh
# Чистка локального Docker на main после сборки (CI, deploy/build-images.sh).
#   deploy/prune-images.sh [N]
# - образы приложения (метка goat-packer=1: api, worker, gateway, runner-*): N последних релизов v* (по умолчанию 3),
#   теги веток branch-* не трогает (их перезаписывает следующий push);
# - базы слайсеров goat-packer-slicer-*: по две последние на слайсер (тег = версия + хэш рецепта). Старую базу,
#   на которой стоит хранимый runner, Docker только раз-тегирует — её слои живут, пока жив runner;
# - осиротевшие слои goat-packer после перезаписи тегов и кэш BuildKit сверх 8 GB.
# Образ, на котором работает контейнер, Docker не удалит — это нормально.
set -eu
keep=${1:-3}

# <repo> <grep-шаблон тега> <сколько оставить>: новые первыми по дате создания.
prune() {
  docker image ls "$1" --format '{{.CreatedAt}}|{{.Tag}}' | grep "|$2" | sort -r | awk -F'|' -v k="$3" 'NR > k { print $2 }' |
    while read -r tag; do docker image rm "$1:$tag" >/dev/null 2>&1 && echo "removed $1:$tag" || true; done
}

for repo in $(docker image ls --filter label=goat-packer=1 --format '{{.Repository}}' | grep '^goat-packer-' | sort -u); do
  prune "$repo" v "$keep"
done
for repo in goat-packer-slicer-bambu goat-packer-slicer-orca; do
  prune "$repo" '' 2
done
docker image prune -f --filter label=goat-packer=1 >/dev/null
# Кэш сборок BuildKit (слои и кэши Gradle/Cargo/bun) растёт без предела: держим не больше 8 GB, старое — вон.
docker builder prune -f --max-used-space 8gb >/dev/null
