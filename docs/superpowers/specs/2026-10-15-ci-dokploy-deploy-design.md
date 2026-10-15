# Общий CI, сборка goat_packer в Woodpecker и деплой через Dokploy

Дата: 2026-10-15. Статус: согласовано, к реализации.

Заменяет ручную установку `deploy/configure.py` + `deploy/start.sh` (сборка образов прямо на сервере
python-скриптами) на сборку в Woodpecker и деплой через Dokploy. Затрагивает три репозитория:
`goat_packer`, `hlab-infra` (инфраструктура), `kdr` (только пути CI).

## Решения

| Вопрос | Решение |
|---|---|
| Где работает goat_packer | В Docker на `main` (Ubuntu 24.04 x86-64, 4 CPU, 16 GB) |
| Чем деплоится | Dokploy на `mbp15`; `main` подключён к нему как remote server |
| Доставка образов | Без реестра: Woodpecker собирает образы прямо в Docker на `main` |
| GitHub Release | Черновик по тегу `v*` (как у `kdr`): заметки + архив CLI под Linux |
| Доступ сейчас | Только владелец: Pangolin с SSO, `goat.ezheg.xyz` и `goat-files.ezheg.xyz` |
| Доступ потом | Свой домен второго уровня через traefik от Dokploy на `main` (отдельная задача) |
| CI | Один Woodpecker `ci.ezheg.xyz` для всех проектов, переименован из `kdr-ci` |
| Docker на `mbp15` | Один colima, один профиль `default` для Dokploy и сборок |

Роли хостов: Pangolin на `cloud` — закрытый вход во внутренние проекты (SSO). `main` — хост будущих
публичных проектов под своими доменами. Dokploy только деплоит: заходит на `main` по SSH, кладёт compose
и env, выполняет `docker compose up`. Контейнеры работают независимо от Dokploy (`restart: unless-stopped`);
без Dokploy недоступно только управление (версия, настройки, логи в UI).

## Часть 1. Инфраструктура

### main

- Стек Woodpecker `~/kdr-ci` → `~/ci`, compose-проект `ci` (server, agent `main-linux`, newt).
  Источник compose — `hlab-infra/ci/docker-compose.yml`; секреты в `~/ci/.env` (не в git).
- Данные сохраняются: при остановленном сервере volumes `kdr-ci_woodpecker-data` и
  `kdr-ci_woodpecker-agent` копируются в `ci_woodpecker-data` / `ci_woodpecker-agent`.
  `WOODPECKER_HOST`, GitHub OAuth app, вебхуки, порты `127.0.0.1:8000/9000` не меняются.
  Старые volumes удаляются только после проверки сборки `kdr` на новом стеке.
- Кэши проектов: `/var/lib/ci/<проект>/`. Для `kdr`: `/var/lib/kdr-ci/*` → `/var/lib/ci/kdr/*`
  (`mirror.git`, `cargo-registry`, `target-linux`); старый путь — симлинк, пока в `kdr` не влиты
  новые пути `.woodpecker/*.yaml` и push-адрес `ssh://main/var/lib/ci/kdr/mirror.git`.
- Метки агентов: `host=main` / `host=mbp15` (в дополнение к `platform`). Пайплайны выбирают машину явно.
- Beszel-агент: стек `hlab-infra/beszel` деплоится через Dokploy на `main` (host network, исходящее
  подключение к `metrics.ezheg.xyz`). Токен системы создаётся в UI Beszel (Settings → Systems → Add system).
- ufw: снаружи только SSH 2783 (`LIMIT`: частые подключения режутся — работать одним мультиплексированным
  SSH-соединением). Docker публикует порты в обход ufw, поэтому все сервисы публикуются только на `127.0.0.1`.

### mbp15

- Один colima: MacPorts `/opt/local/bin/colima` 0.10.3, профиль `default` (4 CPU, 10 GB, 60 GB).
  Одно launchd-задание (`/Library/LaunchDaemons/com.ezheg.colima.plist`, `UserName=devtest`).
  Удаляются: zerobrew colima 0.10.1, LaunchAgent `com.ezheg.colima` (падает с кодом 1), `com.devtest.colima`.
  Перезапуск colima прерывает сервисы hlab примерно на минуту — делать в согласованное время.
- Агенты в `~/ci` (вместо `~/kdr-ci`): `mbp15-mac` (backend local) и `mbp15-docker`
  (backend docker, `DOCKER_HOST=unix:///Users/devtest/.colima/default/docker.sock` — тот же Docker, что у Dokploy).
  launchd: `com.ezheg.ci-agent`, `com.ezheg.ci-agent-docker`, `com.ezheg.ci-tunnel`.
- Туннель `com.ezheg.ci-tunnel` (сейчас завершается с кодом 255, агент Mac отключён с 2026-10-14):
  `KeepAlive`, `ServerAliveInterval`, `ExitOnForwardFailure`;
  `-L 127.0.0.1:19000:127.0.0.1:9000` (gRPC Woodpecker) и
  `-R 127.0.0.1:13000:127.0.0.1:3000` (Dokploy API для шага деплоя на `main`, наружу не открыт).

### Документация

`hlab-infra/ci/README.md`: топология CI, как подключить новый проект (активировать репозиторий в UI,
trusted для volumes/docker socket, секреты), метки агентов, очистка кэшей. Обновить `hlab-infra/CLAUDE.md`
и таблицу хостов в README.

## Часть 2. Сборка goat_packer

### `.woodpecker/check.yaml`

Push в `master`, pull request, manual; `labels: {platform: linux/amd64, host: main}`; клон с GitHub
(зеркало не нужно: клон занимает ~1 с).

- Rust: `cargo test --release --all --locked` (кэш registry/target в `/var/lib/ci/goat_packer/`).
- Backend: `./gradlew test` (Testcontainers через `/var/run/docker.sock`, репозиторий trusted).
- Frontend: `bun test tests`, `typecheck`, `lint`, `build`.
- Очистка кэшей по образцу `kdr`.

### `.woodpecker/release.yaml`

Тег `v*`, `host: main`.

1. Те же проверки, что в `check.yaml`.
2. Образы слайсеров (база): `goat-packer-slicer-bambu:2.8.2.61`, `goat-packer-slicer-orca:2.4.2`.
   Собираются, только если такого тега ещё нет в Docker на `main` (смена версии = смена `runners.lock.json`).
3. Образы версии `vX`: `goat-packer-api`, `goat-packer-worker`, `goat-packer-gateway`,
   `goat-packer-runner-bambu`, `goat-packer-runner-orca`. Runner = база слайсера + бинарники упаковщика (~10 MB).
4. Черновик GitHub Release `vX` (`gh`, секрет `github_token`): `--generate-notes` +
   `goat-packer-vX-linux-x86_64.tar.gz` (`colored_3mf_packer`, `daily_scheduler`, `verify_outputs`).
5. Деплой: Dokploy API через `http://127.0.0.1:13000` (шаг в host network):
   `compose.update` env `GOAT_VERSION=vX`, затем `compose.deploy`, ожидание статуса.
6. Проверка: `deploy/smoke.py` против `127.0.0.1:<порт gateway>` на `main`. При `COOKIE_SECURE=true`
   cookiejar не отправляет secure-cookie по http, поэтому smoke передаёт `X-Forwarded-Proto: https`
   и отправляет cookie сам; проверить это до первого релиза.
7. Хранятся образы трёх последних версий; старые `goat-packer-*:v*` удаляются (диск `main` 50 GB).

### Без python в настройке

- `deploy/fetch-runners.py` → `ADD --checksum=sha256:<hash> <url>` в `Dockerfile.runner`.
- `deploy/runner/extract-appimage.py` → `unsquashfs -o <offset>`; offset SquashFS закреплён рядом
  с хэшем (для текущих AppImage 944632; первый `hsqs` — ложное совпадение).
- `deploy/configure.py`, `deploy/start.sh` удаляются. Их работу делают переменные окружения в Dokploy.
- `deploy/smoke.py` остаётся только как автоматическая проверка в CI.

## Часть 3. Деплой через Dokploy

- `main` подключается в Dokploy как remote server (SSH-ключ Dokploy, `Setup server`). Dokploy создаёт
  `dokploy-network`, включает Swarm и ставит traefik. Traefik на `main` после установки останавливается
  и получает `restart=no`; в Dokploy для этого сервера не нажимать «Reload Traefik» (записать в README).
  Порты 80/443 на `main` закрыты до покупки домена.
- Compose-стек `goat-packer` в Dokploy: источник — GitHub `petrpopov/goat_packer`, `deploy/compose.yml`,
  ветка `master`, autoDeploy выключен (деплой запускает релиз).
- `deploy/compose.yml`: без `build`, только `image: goat-packer-*:${GOAT_VERSION}`, `pull_policy: never`.
  Сервисы публикуются на `127.0.0.1` (порты из env). Значения по умолчанию документированы в compose.
- Переменные в Dokploy: `GOAT_VERSION`; секреты `DATABASE_PASSWORD`, `S3_SECRET_KEY`, `ADMIN_LOGIN`,
  `ADMIN_PASSWORD`; адрес скачиваний `S3_PUBLIC_ENDPOINT`; `COOKIE_SECURE=true`. Адрес приложения
  backend берёт из forwarded-заголовков gateway; отдельная переменная вводится, только если проверка
  CSRF/Origin за Pangolin этого потребует;
  ресурсы `MAX_PARALLEL`, `EXECUTION_CPU_BUDGET`, `EXECUTION_MEMORY_MIB`, `WORKER_CPUS`,
  `WORKER_MEMORY_MIB`, `STORAGE_LIMIT_BYTES` (для `main`: 1 слот, 2 CPU, 4 GiB — как в квалификации);
  `SCRATCH_ROOT`.
- Pangolin (SSO): `goat.ezheg.xyz` → newt на `main` → `127.0.0.1:<WEB_PORT>`;
  `goat-files.ezheg.xyz` → `127.0.0.1:<OBJECT_PORT>`. Проверить, что подписанные ссылки скачивания
  проходят SSO на втором имени; если нет — отдавать файлы через тот же домен, что и приложение.
- Обновление и откат: смена `GOAT_VERSION` в Dokploy + Deploy (релиз делает это сам).
- Переход на свой домен (позже): включить traefik на `main`, домены в Dokploy, Let's Encrypt,
  открыть 80/443 в ufw, сменить `S3_PUBLIC_ENDPOINT`, снять SSO.
- Тестовый стенд `~/goat_packer` на `main` (compose-проект `goat-packer`, только тестовые данные)
  удаляется вместе с volumes после того, как стек из Dokploy пройдёт smoke.

## Проверка готовности

1. `kdr`: сборка на push проходит на новом стеке `ci`, Mac-агент подключён, кэши на новых путях.
2. `goat_packer`: `check.yaml` зелёный; тег `v0.1.0` даёт черновик Release с архивом CLI и
   рабочий деплой; smoke проходит.
3. `goat.ezheg.xyz` открывается после SSO, упаковка реального проекта и скачивание 3MF работают.
4. На `main` снаружи открыт только SSH (`ss -ltn` + проверка портов извне); 80/443 закрыты.
5. `main` виден в Beszel.
6. На `mbp15` один colima и один профиль; сервисы hlab работают.

## Риски

- Swarm на `main` от Dokploy: Woodpecker и compose-стеки продолжают работать без Swarm-режима,
  проверить после `Setup server`.
- Dokploy может пересоздать traefik при действиях с сервером — отслеживать порты после изменений.
- Перенос volume Woodpecker: делать при остановленном сервере, старые volumes хранить до проверки.
