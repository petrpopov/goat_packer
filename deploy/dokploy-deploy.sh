#!/bin/sh
# CI-шаг релиза: переключить стек goat-packer в Dokploy на версию тега и дождаться деплоя.
# Нужны DOKPLOY_TOKEN, COMPOSE_ID, CI_COMMIT_TAG; DOKPLOY_API по умолчанию — туннель CI на main.
set -eu
api=${DOKPLOY_API:-http://127.0.0.1:13000/api}
get() { curl -fsS -H "x-api-key: $DOKPLOY_TOKEN" "$api/$1?composeId=$COMPOSE_ID"; }
post() { curl -fsS -X POST -H "x-api-key: $DOKPLOY_TOKEN" -H 'Content-Type: application/json' -d "$2" "$api/$1" >/dev/null; }
env=$(get compose.one | jq -r '.env // ""' | grep -v '^GOAT_VERSION=' || true)
post compose.update "$(jq -n --arg id "$COMPOSE_ID" --arg env "$env
GOAT_VERSION=$CI_COMMIT_TAG" '{composeId: $id, env: $env}')"
post compose.deploy "$(jq -n --arg id "$COMPOSE_ID" '{composeId: $id}')"
echo "Dokploy: деплой $CI_COMMIT_TAG запущен"
sleep 5
for _ in $(seq 120); do
  case $(get compose.one | jq -r .composeStatus) in
    done) echo "Dokploy: $CI_COMMIT_TAG развёрнут"; exit 0 ;;
    error) echo "Dokploy: деплой $CI_COMMIT_TAG завершился ошибкой — журнал деплоя в Dokploy" >&2; exit 1 ;;
  esac
  sleep 5
done
echo "Dokploy: деплой $CI_COMMIT_TAG не завершился за 10 минут" >&2; exit 1
