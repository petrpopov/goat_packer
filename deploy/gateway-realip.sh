#!/bin/sh
# Entrypoint-скрипт gateway (/docker-entrypoint.d): GOAT_TRUSTED_UPSTREAM — адреса/CIDR через пробел,
# от которых X-Forwarded-For считается правдой. За туннелем Pangolin (newt → 127.0.0.1:WEB_PORT) все
# запросы приходят с шлюза Docker-сети: без этого лимиты по IP (nginx и бэкенда) — одни на всех.
# Пусто — клиентский адрес соединения (прямой доступ: заголовок клиента подделал бы адрес).
set -eu
mkdir -p /etc/nginx/goat
out=/etc/nginx/goat/realip.conf
rm -f "$out"
[ -n "${GOAT_TRUSTED_UPSTREAM:-}" ] || exit 0
for cidr in $GOAT_TRUSTED_UPSTREAM; do echo "set_real_ip_from $cidr;"; done > "$out"
# Справа налево до первого недоверенного: подставленный клиентом слева адрес не используется.
printf 'real_ip_header X-Forwarded-For;\nreal_ip_recursive on;\n' >> "$out"
echo "goat gateway: X-Forwarded-For trusted from $GOAT_TRUSTED_UPSTREAM"
