# Сборка и развёртывание

Образы собирает Woodpecker (`ci.ezheg.xyz`) прямо в Docker на `main`; реестра нет.

- Push в любую ветку: `.woodpecker/check.yaml` — тесты Rust, backend (Testcontainers), frontend и `lint`
  (shellcheck скриптов `deploy/`, `deploy/check-pins.sh`), затем образы `goat-packer-*:branch-<ветка>`
  (перезаписываются следующим push, никуда не выкладываются).
- Тег `v*`: `.woodpecker/release.yaml` — `deploy/build-images.sh <тег>` собирает
  `goat-packer-{api,worker,gateway,runner-bambu,runner-orca}:<тег>`, черновик GitHub Release получает
  `goat-packer-<тег>-linux-x86_64.tar.gz` (CLI), затем деплой в Dokploy и smoke (см. ниже).
  Хранятся образы трёх последних версий (`deploy/prune-images.sh`).

Выпуск версии: `git tag v0.2.0 && git push origin v0.2.0`. Черновик релиза публикуется руками:
GitHub → Releases → черновик → Edit → Publish release (на работу сервера публикация не влияет).

Базы слайсеров `goat-packer-slicer-{bambu,orca}:<версия>-<хэш рецепта>` описаны в `deploy/slicers.env`
(URL, SHA256, offset SquashFS) и собираются один раз; AppImage при сборке не запускается. Смена версии
слайсера — правка `slicers.env`, следующая сборка соберёт новую базу; правка `Dockerfile.slicer`,
`runner/slicer.sh` или `apt-mirror.sh` меняет хэш и тоже пересобирает базу. Хранятся две последние базы
на слайсер.

Руками на x86-64 машине с Docker: `sh deploy/build-images.sh dev`, затем
`GOAT_VERSION=dev docker compose --env-file deploy/.env -f deploy/compose.yml up -d`
(в `deploy/.env` — `DATABASE_PASSWORD`, `S3_SECRET_KEY`, `ADMIN_PASSWORD` и параметры ниже).
Развёртывание на сервер — через Dokploy, см. следующий раздел.

## Файлы

| Файл | Что делает |
|---|---|
| `build-images.sh` | все образы версии: базы слайсеров (кэш AppImage `SLICER_CACHE`), runner, api/worker, gateway |
| `slicers.env` | закреплённые сборки слайсеров |
| `Dockerfile.slicer` | база слайсера: распакованный AppImage + Xvfb, пользователь 1000 |
| `Dockerfile.runner` | база слайсера + бинарники упаковщика релиза |
| `Dockerfile.backend` | `api` и `worker` (тот же образ + docker CLI) |
| `Dockerfile.frontend` | `gateway`: статика фронтенда + nginx (`nginx.conf`, `gateway-realip.sh`) |
| `apt-mirror.sh` | `APT_MIRROR` для apt внутри сборки (в образ не попадает) |
| `compose.yml` | стек: postgres, storage (SeaweedFS), api, worker, gateway |
| `dokploy-deploy.sh`, `smoke.py` | шаг деплоя релиза и проверка стека |
| `prune-images.sh` | чистка старых версий, баз и кэша BuildKit на `main` |
| `check-pins.sh` | версии Rust/JDK/bun/docker CLI в `.woodpecker` и Dockerfile'ах совпадают |
| `catalog-qualify.sh` | квалификация каталога принтеров в runner-образах (bash, вручную) |

Закреплённые версии: Rust, JDK, bun и docker CLI — в `FROM` Dockerfile'ов и в `image:` шагов CI
(Woodpecker не читает их из файла; расхождение ловит `check-pins.sh`). Слайсеры — `slicers.env`,
SeaweedFS — по digest в `compose.yml`.

## Развёртывание через Dokploy (`main`)

- **Сервер** `main` в Dokploy (на `mbp15`): root по ключу Dokploy и только с домашнего адреса
  (`sshd_config.d/90-dokploy.conf`, `from=` в `/root/.ssh/authorized_keys`, ufw-правило для этого адреса).
  Установка урезана: Swarm и `dokploy-network`, без сборщиков (Nixpacks/Railpack/Buildpacks) и **без traefik** —
  порты 80/443 на `main` закрыты. Ежедневная чистка Docker от Dokploy **выключена**: она удалила бы локальные
  образы (`kdr-ci-linux:1`, базы слайсеров, версии для отката). Кнопку «Reload Traefik» для `main` не нажимать.
- **Стек** `goat-packer` (composeId `9extEhTelEZyZ7NbPJS1y`): источник — этот репозиторий, ветка `master`,
  `deploy/compose.yml`, автодеплой выключен. Переменные — в Dokploy: `GOAT_VERSION`, пароли, `COOKIE_SECURE=true`,
  `GOAT_PUBLIC_SCHEME=https`, `ADMISSION_LIMITS=false` (сайт закрыт SSO Pangolin — лимитов частоты,
  активных задач, бюджета и личной квоты нет; общий потолок хранилища остаётся),
  `S3_PUBLIC_ENDPOINT=https://goat.ezheg.xyz`, `WEB_PORT=18088`, `WORKER_ID=main-1`,
  `SCRATCH_ROOT=/srv/goat-packer/scratch`, ресурсы (1 слот: 2 CPU, 4 GiB).
- **Доступ**: Pangolin `goat.ezheg.xyz` (SSO) → newt на `main` → `127.0.0.1:18088`. Файлы идут по `/goat/` того же имени.
- **Релиз** = деплой: тег `v*` → образы → `deploy/dokploy-deploy.sh` (ключ Dokploy в
  `/var/lib/ci/goat_packer/secrets/dokploy.env` на `main`, root 600; API через туннель CI `127.0.0.1:13000`)
  → `deploy/smoke.py` против `127.0.0.1:18088`.
- **Откат**: в Dokploy сменить `GOAT_VERSION` на предыдущий тег (хранятся три последние) → Deploy.
- **Открыть сайт наружу** (без SSO): вернуть лимиты (`ADMISSION_LIMITS=true`) и задать `GOAT_TRUSTED_UPSTREAM=172.16.0.0/12` —
  newt приходит в gateway с шлюза Docker-сети, без этого лимиты по IP (nginx и бэкенда) общие на всех.
- **Свой домен** (позже): включить traefik на `main`, привязать домен к сервису `gateway` в Dokploy
  (Let's Encrypt), открыть 80/443 в ufw, `S3_PUBLIC_ENDPOINT=https://<домен>`, снять SSO/ресурс Pangolin.

Обработка задач идёт локально в контейнерах **без сети**; слайсеры на хосте не нужны.
При HTTP пароли и cookies передаются без шифрования.

Кэши CI на `main` — `/var/lib/ci/goat_packer/`: `cargo-registry`, `target` (Cargo, чистится сверх 10 ГБ),
`gradle`, `slicers` (скачанные AppImage: GitHub с `main` медленный), `secrets` (ключ Dokploy).

## Контейнеры стека

Все сервисы: `restart: unless-stopped`, `no-new-privileges`, лимит памяти, логи 10 MiB × 3.

| Сервис | Порт | Пользователь, права | Healthcheck |
|---|---|---|---|
| `postgres` | 5432 (внутри) | образ postgres | `pg_isready` |
| `storage` | 8333 (внутри, S3) | образ SeaweedFS | `/healthz` |
| `api` | 19190 (внутри) | uid 1000, read-only, cap_drop ALL | `/api/v2/health` |
| `worker` | — | root + сокет Docker хоста, read-only | свежий файл `ready` в scratch |
| `gateway` | `${BIND_ADDRESS:-127.0.0.1}:${WEB_PORT:-8088}` → 80 | мастер nginx root с 4 capabilities, read-only | `GET /` |

Worker с сокетом Docker равен root на хосте, поэтому его права не урезаются: изолированы задачи.
`SCRATCH_ROOT` — путь на хосте: worker монтирует его в `/scratch` и им же (`HOST_WORK_ROOT`) задаёт тома
sandbox-контейнеров через сокет хоста.

## Ресурсы и исполнение

По умолчанию одна native-задача: 2 CPU, 4 GiB RAM, жёсткий tmpfs scratch 2 GiB, 256 процессов,
30 мин максимум, 50 MiB совокупных выходных файлов (включая диагностику). Export helper ограничен
128 MiB / 0.5 CPU; запас control plane учтён в расчёте. Пакер ограничивает Rayon двумя потоками.
Авторасчёт использует CPU/RAM **Docker-хоста**, не предполагает фиксированную машину 4 GiB.
На shared-host явно укажите выделенный бюджет. Нулевое безопасное число слотов допустимо:
система не начинает задачи вместо рискованного принудительного N=1.

Worker — доверенный контроллер с Docker socket. Он не выполняет загруженный код в своей JVM.
Каждая задача имеет отдельный непривилегированный контейнер (uid 1000): read-only root, network none,
cap-drop ALL, no-new-privileges, CPU/RAM/PID limits, собственный ограниченный tmpfs volume.
Никаких credentials/socket/host HOME внутри него. После расчёта контейнер замораживается;
отдельный доверенный read-only helper экспортирует только разрешённые обычные файлы с
проверкой размера и запретом symlink/hardlink. Затем sandbox/volume удаляются.

Один controller JVM обслуживает N параллельных sandbox-контейнеров. Не запускайте
`--scale worker` с одним WORKER_ID: OS file lock это запрещает. Дополнительный execution-host
требует уникального WORKER_ID, собственного постоянного scratch и выделенного бюджета.
Не переносите WORKER_ID на другой Docker daemon без подтверждённого выключения старого.

После аварийного рестарта worker сначала удаляет старые контейнеры своего node, подтверждает
остановку, затем возвращает задачи в `RETRY_WAIT`. Один повтор максимум, новый fencing token,
тот же закреплённый image ID; старый процесс не может публиковать результаты. При недоступном
Docker/неподтверждённом kill слот остаётся LOST, а не запускается дубликат.

## Лимиты публичного сервиса

Действуют при `ADMISSION_LIMITS=true` (по умолчанию; в проде за SSO выключены, общий потолок хранилища
остаётся всегда).

- Очередь максимум 20 задач; на гостя 2 активных, аккаунт 4; одновременно одна задача владельца.
- CPU-time admission: резерв полного timeout; по умолчанию гость 3600 с/сутки,
  аккаунт 10800 с/сутки, глобально 72000 с/сутки. Неиспользованный резерв возвращается.
- Гость: 150 MiB и 24 ч хранения после окончания; аккаунт 1 GiB без TTL полезных результатов.
  JSON/log аккаунтов удаляются через 7 дней. Общая storage quota ограничивает admission.
- До загрузки одноразовый owner-bound SHA256 proof-of-work в браузерном Web Worker.
  Работает и по HTTP/IP, без сторонних CAPTCHA/CDN. Есть общие/IP/owner rate limits,
  максимум 2 одновременных upload intents, 50 MiB вход, временной лимит загрузки и 64 KiB control JSON.
- ZIP traversal/дубликаты путей/DTD/external entities/post-process commands отклоняются;
  лимит распакованного содержимого 1 GiB, XML-entry 256 MiB, 10000 entries.
- Это ограничение ущерба, а не гарантия от распределённого DDoS. Без регистрации нельзя
  надёжно связать разные гостевые cookies с одним человеком; блокировка гостя блокирует
  его principal/сессии, но не все будущие анонимные личности. Объёмные сетевые атаки остаются
  за пределами приложения.

Завершённые файлы отдаются краткоживущими signed S3 URL. Bucket закрыт, GET/HEAD gateway
не разрешает запись/listing. SHA256-dedup не раскрывает другим пользователям наличие файла.
Удаление reference-aware: общий blob живёт, пока нужен хотя бы одной загрузке/задаче.
При неопределённом исходе DB commit файлы не удаляются немедленно: durable GC устраняет
осиротевшие stages, не затрагивая опубликованные результаты.

## Наблюдаемость и проверка

```sh
docker compose --env-file deploy/.env -f deploy/compose.yml ps
docker compose --env-file deploy/.env -f deploy/compose.yml logs --tail 100 api worker
python3 deploy/smoke.py --url http://127.0.0.1:8088
```

Smoke создаёт гостевые задачи на синтетической модели **из репозитория**, выполняет реальные
pack→schedule, скачивает 3MF из S3, проверяет скрытие диагностики и удаляет свои тестовые задачи.
Не удаляет существующие данные. Для внешнего запуска укажите IP вместо 127.0.0.1.

Backend logs — JSON/ECS с jobId/attemptFence; события slicer stdout/stderr передаются в job log
без неограниченного буфера. В обычном UI видны 3MF/таблицы, при `?debug=true` — ещё JSON/log.
Админка показывает пользователей/гостей, блокировку, одноразовый сброс пароля, фактические/разрешённые
слоты и состояние worker.

```sh
TESTCONTAINERS_DOCKER_SOCKET_OVERRIDE=/var/run/docker.sock ./webapp/backend/gradlew -p webapp/backend test
./webapp/backend/gradlew -p webapp/backend nativeTest  # нужны собранные runner images
cd webapp/frontend && bun test tests && bun run typecheck && bun run lint && bun run build
sh deploy/check-pins.sh   # и shellcheck -x deploy/*.sh deploy/runner/slicer.sh, как шаг lint
```

Backend tests используют отдельные disposable PG/S3, не рабочую БД. `nativeTest` не скрывается
как успешный при отсутствии образов: их нужно собрать. Fixtures не ссылаются на Downloads.
Старый профиль `local` остаётся только для доверенного локального использования.

Gateway сохраняет внешний порт в X-Forwarded-Port, перезаписывает клиентские forwarded headers
и не отключает CORS/CSRF. Ошибка 403 на localhost:8088 проверяется настоящим nginx/Tomcat тестом.
За прокси, снимающим TLS (Pangolin), gateway получает `GOAT_PUBLIC_SCHEME=https` — схема задаётся
конфигурацией, клиентский `X-Forwarded-Proto` на неё не влияет. Подписанные ссылки на файлы идут по пути
`/goat/<ключ>` того же имени (`S3_PUBLIC_ENDPOINT` = публичный адрес приложения): gateway отдаёт его
хранилищу только на чтение, без изменения Host и пути, поэтому подпись S3 сходится. Отдельное имя
или порт для файлов не нужны.

### Параметры

Основные env: `WORKER_CPUS`, `WORKER_MEMORY_MIB`, `EXECUTION_CPU_BUDGET`,
`EXECUTION_MEMORY_MIB`, `MAX_PARALLEL` (начальный), `STORAGE_LIMIT_BYTES`, `JOB_TIMEOUT_SECONDS`,
`MAX_QUEUE`, `GLOBAL_COMPUTE_SECONDS`, `GOAT_VERSION` (версия образов, runner — той же версии).
Сеть и пути: `WEB_PORT`, `BIND_ADDRESS`, `SCRATCH_ROOT`, `DOCKER_SOCKET`, `S3_PUBLIC_ENDPOINT`,
`GOAT_PUBLIC_SCHEME`, `GOAT_TRUSTED_UPSTREAM`. Сборка: `APT_MIRROR`, `SLICER_CACHE`.
Снижение лимита параллелизма в админке останавливает новые запуски, но не убивает уже работающие
задачи. ETA появляется после пяти сопоставимых успешных измерений и является диапазоном,
а не гарантированным сроком. Не повышайте memory/parallelism без выделенного ресурса.
