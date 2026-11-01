#!/bin/sh
# Переключает apt образа (Debian/Ubuntu) на зеркало APT_MIRROR, например http://mirror.yandex.ru; пусто — ничего.
# Вызывается из Dockerfile'ов через RUN --mount=type=bind (в образ не попадает). CI на main передаёт
# mirror.yandex.ru: deb.debian.org и archive.ubuntu.com оттуда отдают 0.3–5 MB/s против ~30 MB/s.
set -eu
[ -n "${APT_MIRROR:-}" ] || exit 0
find /etc/apt \( -name '*.list' -o -name '*.sources' \) -exec sed -i -E \
  "s#https?://(deb\.debian\.org|archive\.ubuntu\.com|security\.ubuntu\.com)#$APT_MIRROR#g" {} +
