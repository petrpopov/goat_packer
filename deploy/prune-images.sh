#!/bin/sh
# Оставляет N последних версий образов приложения (по умолчанию 3). Базы слайсеров не трогает.
# Образ, на котором работает контейнер, Docker не удалит — это нормально.
set -eu
keep=${1:-3}
for repo in goat-packer-api goat-packer-worker goat-packer-gateway goat-packer-runner-bambu goat-packer-runner-orca; do
  docker image ls "$repo" --format '{{.CreatedAt}}|{{.Tag}}' | sort -r | awk -F'|' -v k="$keep" 'NR > k { print $2 }' |
    while read -r tag; do docker image rm "$repo:$tag" >/dev/null 2>&1 && echo "removed $repo:$tag" || true; done
done
