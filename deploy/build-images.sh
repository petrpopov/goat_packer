#!/bin/sh
# Собирает все образы goat-packer версии в локальный Docker (CI на main вызывает его же).
#   deploy/build-images.sh v0.1.0
# APT_MIRROR (например http://mirror.yandex.ru) — зеркало Debian/Ubuntu для apt внутри сборки; пусто = официальные.
# SLICER_CACHE — каталог скачанных AppImage (CI: /var/lib/ci/goat_packer/slicers); качаются один раз на версию.
set -eu
version=${1:?usage: deploy/build-images.sh <version>}
cd "$(dirname "$0")/.."
. deploy/slicers.env
cache=${SLICER_CACHE:-$HOME/.cache/goat-packer/slicers}
mkdir -p "$cache"
build() { docker build --platform linux/amd64 --build-arg APT_MIRROR="${APT_MIRROR:-}" "$@" .; }

# Скачать в кэш и проверить SHA256; битый или чужой файл не используется.
fetch() { # <file> <url> <sha256>
  if [ -f "$cache/$1" ] && echo "$3  $cache/$1" | sha256sum -c - >/dev/null 2>&1; then return; fi
  curl -fL --retry 3 -o "$cache/$1.part" "$2"
  echo "$3  $cache/$1.part" | sha256sum -c -
  mv "$cache/$1.part" "$cache/$1"
}

# Тег базы включает хэш её рецепта: правка Dockerfile.slicer или slicer.sh пересобирает базу и без смены версии.
recipe=$(cat deploy/Dockerfile.slicer deploy/runner/slicer.sh | sha256sum | cut -c1-8)

for kind in bambu orca; do
  case $kind in
    bambu) v=$BAMBU_VERSION url=$BAMBU_URL sha=$BAMBU_SHA256 off=$BAMBU_SQUASHFS_OFFSET ;;
    orca)  v=$ORCA_VERSION url=$ORCA_URL sha=$ORCA_SHA256 off=$ORCA_SQUASHFS_OFFSET ;;
  esac
  base=goat-packer-slicer-$kind:$v-$recipe
  if ! docker image inspect "$base" >/dev/null 2>&1; then
    file=$kind-$v.AppImage
    fetch "$file" "$url" "$sha"
    build -f deploy/Dockerfile.slicer --build-context slicer="$cache" --build-arg SLICER_FILE="$file" \
      --build-arg SLICER_SHA256="$sha" --build-arg SQUASHFS_OFFSET="$off" -t "$base"
  fi
  build -f deploy/Dockerfile.runner --build-arg SLICER_IMAGE="$base" -t "goat-packer-runner-$kind:$version"
done
build -f deploy/Dockerfile.backend --target api -t "goat-packer-api:$version"
build -f deploy/Dockerfile.backend --target worker -t "goat-packer-worker:$version"
build -f deploy/Dockerfile.frontend -t "goat-packer-gateway:$version"
docker image ls --filter "reference=goat-packer-*:$version"
