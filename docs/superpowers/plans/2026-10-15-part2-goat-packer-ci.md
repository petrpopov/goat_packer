# Часть 2. Сборка goat_packer в Woodpecker — план реализации

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** push в любую ветку прогоняет все тесты `goat_packer` в Woodpecker на `main`; тег `v*` собирает образы `goat-packer-*:vX` прямо в Docker на `main` и создаёт черновик GitHub Release с CLI под Linux. Python в настройке не остаётся.

**Architecture:** Спецификация `docs/superpowers/specs/2026-10-15-ci-dokploy-deploy-design.md`, часть 2. Один shell-скрипт `deploy/build-images.sh <версия>` — единственное место, где описана сборка образов: его вызывает CI, им же можно собрать руками. Образ слайсера делится на базу (`Dockerfile.slicer`, собирается только при смене версии слайсера) и тонкий runner на релиз (`Dockerfile.runner` = база + бинарники упаковщика). `deploy/compose.yml` больше ничего не собирает, только запускает `goat-packer-*:${GOAT_VERSION}`. Деплой и smoke в релизе — часть 3.

**Tech Stack:** Woodpecker 3.19 (docker backend, агент `main-linux`, метка `host=main`), Docker 29 + BuildKit, rust:1.94.0, eclipse-temurin:21, oven/bun:1.3.14, docker:29-cli, alpine + github-cli.

---

## Файлы

| Файл | Что делает |
|---|---|
| `deploy/slicers.env` (новый) | Версии, URL, SHA256 и offset SquashFS закреплённых AppImage. Заменяет `runners.lock.json` |
| `deploy/Dockerfile.slicer` (новый) | База: скачать AppImage, проверить SHA256, распаковать SquashFS, runtime Ubuntu 24.04 + Xvfb |
| `deploy/Dockerfile.runner` (переписать) | База слайсера + `colored_3mf_packer`, `daily_scheduler`, `verify_outputs` этого коммита |
| `deploy/build-images.sh` (новый) | Сборка всех образов версии; базы — только если их нет |
| `deploy/prune-images.sh` (новый) | Оставить N последних версий образов приложения |
| `deploy/compose.yml` (изменить) | Только `image: …:${GOAT_VERSION}`, без `build` и без сервисов `runner-*` |
| `.woodpecker/check.yaml` (новый) | Тесты Rust, backend, frontend |
| `.woodpecker/release.yaml` (новый) | Образы, архив CLI, черновик Release, очистка образов |
| `.dockerignore` (изменить) | Исключить `deploy/vendor`, `docs`, `examples` |
| `deploy/README.md` (изменить) | Раздел сборки вместо `configure.py`/`start.sh` |
| Удалить | `deploy/configure.py`, `deploy/fetch-runners.py`, `deploy/runner/extract-appimage.py`, `deploy/start.sh`, `deploy/runners.lock.json` |

## Факты

- Ветка `production-deploy` (не отправлена на GitHub). Репозиторий `petrpopov/goat_packer` публичный, клон с `main` занимает ~1 с — зеркало не нужно.
- Offset SquashFS в обоих закреплённых AppImage — `944632` (первый `hsqs` в файле — ложное совпадение; offset проверен разбором суперблока). Каждый Dockerfile дополнительно проверяет, что по offset лежит `hsqs`.
- Backend собирается в `build/libs/*-1.0.0.jar` (`version = "1.0.0"` в `webapp/backend/build.gradle.kts`); `Dockerfile.backend` уже копирует этот jar — не меняем.
- Backend-тесты используют Testcontainers (PostgreSQL, SeaweedFS `chrislusf/seaweedfs:4.47`, `nginx:1.28-alpine`); `docker-java.properties` фиксирует `api.version=1.44`. В CI нужен `/var/run/docker.sock` → репозиторий trusted (volumes).
- Тег `nativeTest` (настоящие слайсеры) в CI не запускается: по умолчанию исключён из `test`.
- `main` 50 GB диска: база слайсера ~1.5 GB, runner на релиз ~10 MB поверх базы, api/worker/gateway ~0.6/0.9/0.1 GB.
- SSH на `main` — только через мультиплексированное соединение (`-o ControlPath=/tmp/main.sock`, см. часть 1).
- Синхронизация рабочей копии (включая незакоммиченные файлы, без игнорируемых) в `~/gp-build` на `main` для тестовых сборок — далее «sync»:

  ```bash
  git ls-files -co --exclude-standard | tar -cz -T - | ssh -o ControlPath=/tmp/main.sock main 'rm -rf ~/gp-build && mkdir ~/gp-build && tar -xz -C ~/gp-build'
  ```

## Точки согласования

Задача 9 (активация репозитория в Woodpecker, секрет `github_token`) делается владельцем в UI. Перед `git push` ветки, слиянием в `master` и тегом — явное «да».

---

### Task 1: `deploy/slicers.env` и база слайсера

**Files:**
- Create: `deploy/slicers.env`
- Create: `deploy/Dockerfile.slicer`

- [ ] **Step 1: `deploy/slicers.env`**

```sh
# Закреплённые сборки слайсеров для образов goat-packer-slicer-<kind>:<VERSION> (deploy/Dockerfile.slicer).
# Смена версии: новый URL и SHA256 с релизной страницы, offset SquashFS — проверить на файле:
#   первый суперблок "hsqs" с версией 4.0 (у текущих AppImage первое совпадение "hsqs" ложное).
BAMBU_VERSION=2.8.2.61
BAMBU_URL=https://github.com/bambulab/BambuStudio/releases/download/v02.08.02.61/BambuStudio_ubuntu24.04-v02.08.02.61-20260820225108.AppImage
BAMBU_SHA256=d501b103fac5424513ec0e8d6bc145fb30719de2c7d94d7320d723740c81a7fd
BAMBU_SQUASHFS_OFFSET=944632
ORCA_VERSION=2.4.2
ORCA_URL=https://github.com/OrcaSlicer/OrcaSlicer/releases/download/v2.4.2/OrcaSlicer_Linux_AppImage_Ubuntu2404_V2.4.2.AppImage
ORCA_SHA256=d12fb8c8eac1aecd2dfb6377acd48f994f8fa439ed5292fa532dd82880f029fd
ORCA_SQUASHFS_OFFSET=944632
```

- [ ] **Step 2: `deploy/Dockerfile.slicer`**

```dockerfile
# База goat-packer-slicer-<kind>:<version>: один закреплённый слайсер (Bambu Studio или OrcaSlicer) для linux/amd64.
# AppImage не запускается при сборке: SquashFS распаковывается по закреплённому offset (работает и на ARM-сборщике).
# Пересобирается только при смене deploy/slicers.env; бинарники упаковщика добавляет Dockerfile.runner.
FROM debian:bookworm-slim AS extract
ARG SLICER_URL
ARG SLICER_SHA256
ARG SQUASHFS_OFFSET
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl squashfs-tools \
    && rm -rf /var/lib/apt/lists/*
RUN curl -fsSL --retry 3 "$SLICER_URL" -o /tmp/slicer.AppImage \
    && echo "$SLICER_SHA256  /tmp/slicer.AppImage" | sha256sum -c - \
    && test "$(dd if=/tmp/slicer.AppImage bs=1 skip="$SQUASHFS_OFFSET" count=4 2>/dev/null)" = hsqs \
    && unsquashfs -processors 2 -o "$SQUASHFS_OFFSET" -d /opt/slicer /tmp/slicer.AppImage \
    && rm /tmp/slicer.AppImage

FROM ubuntu:24.04
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    ca-certificates libgtk-3-0t64 libwebkit2gtk-4.1-0 libglu1-mesa libgl1 libegl1 libopengl0 \
    libgstreamer1.0-0 libgstreamer-plugins-base1.0-0 libxrender1 libxi6 libxrandr2 libxinerama1 \
    libxss1 libnss3 libsecret-1-0 libnotify4 libasound2t64 libfuse2t64 libxcb-cursor0 \
    locales fonts-dejavu-core xvfb xauth dbus-x11 tini procps coreutils \
    && rm -rf /var/lib/apt/lists/* && locale-gen en_US.UTF-8
RUN getent passwd 1000 >/dev/null || useradd -m -u 1000 runner
COPY --from=extract /opt/slicer /opt/slicer
COPY deploy/runner/slicer.sh /usr/local/bin/slicer
# Профили принтеров указывают /opt/slicers/<kind>/AppRun; оба пути ведут в обёртку с приватным Xvfb.
RUN mkdir -p /opt/slicers/bambu /opt/slicers/orca \
    && ln -s /usr/local/bin/slicer /opt/slicers/bambu/AppRun \
    && ln -s /usr/local/bin/slicer /opt/slicers/orca/AppRun && chmod +x /usr/local/bin/slicer
ENV HOME=/work/home LANG=en_US.UTF-8 LC_ALL=en_US.UTF-8 LIBGL_ALWAYS_SOFTWARE=1
WORKDIR /work
USER 1000:1000
ENTRYPOINT []
CMD ["/usr/bin/sleep", "infinity"]
```

- [ ] **Step 3: Собрать базу Bambu на `main` и убедиться, что слайсер стартует**

```bash
git ls-files -co --exclude-standard | tar -cz -T - | ssh -o ControlPath=/tmp/main.sock main 'rm -rf ~/gp-build && mkdir ~/gp-build && tar -xz -C ~/gp-build'
ssh -o ControlPath=/tmp/main.sock main 'cd ~/gp-build && . deploy/slicers.env && docker build --platform linux/amd64 -f deploy/Dockerfile.slicer \
  --build-arg SLICER_URL=$BAMBU_URL --build-arg SLICER_SHA256=$BAMBU_SHA256 --build-arg SQUASHFS_OFFSET=$BAMBU_SQUASHFS_OFFSET \
  -t goat-packer-slicer-bambu:$BAMBU_VERSION . 2>&1 | tail -3
docker run --rm --network none goat-packer-slicer-bambu:$BAMBU_VERSION timeout 120 slicer --help 2>&1 | grep -i -m1 -E "usage|bambu"'
```

Expected: сборка успешна; последняя команда печатает строку справки Bambu Studio (`Usage` или `BambuStudio`). Если `sha256sum` или проверка `hsqs` падают — сборка останавливается; это и есть защита.

- [ ] **Step 4: Commit**

```bash
git add deploy/slicers.env deploy/Dockerfile.slicer
git commit -m "Build pinned slicer base images from slicers.env without python"
```

### Task 2: Тонкий runner на релиз

**Files:**
- Modify (переписать целиком): `deploy/Dockerfile.runner`

- [ ] **Step 1: `deploy/Dockerfile.runner`**

```dockerfile
# Runner задачи на релиз: база слайсера (Dockerfile.slicer) + бинарники упаковщика этого коммита (~10 MB слой).
# Собирается на x86-64 (CI на main); glibc bookworm старше Ubuntu 24.04 базы, поэтому бинарники совместимы.
ARG SLICER_IMAGE
FROM rust:1.94.0-bookworm AS packer
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src
ENV CARGO_BUILD_JOBS=3
RUN cargo build --locked --release --bins

FROM ${SLICER_IMAGE}
COPY --from=packer /build/target/release/colored_3mf_packer /build/target/release/daily_scheduler \
     /build/target/release/verify_outputs /usr/local/bin/
```

- [ ] **Step 2: Собрать runner на `main` и проверить бинарники**

```bash
git ls-files -co --exclude-standard | tar -cz -T - | ssh -o ControlPath=/tmp/main.sock main 'rm -rf ~/gp-build && mkdir ~/gp-build && tar -xz -C ~/gp-build'
ssh -o ControlPath=/tmp/main.sock main 'cd ~/gp-build && . deploy/slicers.env && docker build --platform linux/amd64 -f deploy/Dockerfile.runner \
  --build-arg SLICER_IMAGE=goat-packer-slicer-bambu:$BAMBU_VERSION -t goat-packer-runner-bambu:plan-test . 2>&1 | tail -2
docker run --rm --network none goat-packer-runner-bambu:plan-test sh -c "ls -la /usr/local/bin/colored_3mf_packer /usr/local/bin/daily_scheduler /usr/local/bin/verify_outputs; colored_3mf_packer 2>&1 | head -1"
docker history goat-packer-runner-bambu:plan-test --format "{{.Size}}" | head -1'
```

Expected: три исполняемых файла; `colored_3mf_packer` без аргументов печатает ошибку использования (а не `not found` и не ошибку загрузки библиотек); верхний слой около 10–20 MB.

- [ ] **Step 3: Commit**

```bash
git add deploy/Dockerfile.runner
git commit -m "Thin per-release runner image on top of the pinned slicer base"
```

### Task 3: `deploy/build-images.sh` и `deploy/prune-images.sh`

**Files:**
- Create: `deploy/build-images.sh`
- Create: `deploy/prune-images.sh`
- Modify: `.dockerignore`

- [ ] **Step 1: `deploy/build-images.sh`**

```sh
#!/bin/sh
# Собирает все образы goat-packer версии в локальный Docker (CI на main вызывает его же).
#   deploy/build-images.sh v0.1.0
# Базы слайсеров goat-packer-slicer-<kind>:<версия слайсера> собираются, только если их ещё нет.
set -eu
version=${1:?usage: deploy/build-images.sh <version>}
cd "$(dirname "$0")/.."
. deploy/slicers.env
build() { docker build --platform linux/amd64 "$@" .; }

for kind in bambu orca; do
  case $kind in
    bambu) v=$BAMBU_VERSION url=$BAMBU_URL sha=$BAMBU_SHA256 off=$BAMBU_SQUASHFS_OFFSET ;;
    orca)  v=$ORCA_VERSION url=$ORCA_URL sha=$ORCA_SHA256 off=$ORCA_SQUASHFS_OFFSET ;;
  esac
  base=goat-packer-slicer-$kind:$v
  if ! docker image inspect "$base" >/dev/null 2>&1; then
    build -f deploy/Dockerfile.slicer --build-arg SLICER_URL="$url" --build-arg SLICER_SHA256="$sha" \
      --build-arg SQUASHFS_OFFSET="$off" -t "$base"
  fi
  build -f deploy/Dockerfile.runner --build-arg SLICER_IMAGE="$base" -t "goat-packer-runner-$kind:$version"
done
build -f deploy/Dockerfile.backend --target api -t "goat-packer-api:$version"
build -f deploy/Dockerfile.backend --target worker -t "goat-packer-worker:$version"
build -f deploy/Dockerfile.frontend -t "goat-packer-gateway:$version"
docker image ls --filter "reference=goat-packer-*:$version"
```

- [ ] **Step 2: `deploy/prune-images.sh`**

```sh
#!/bin/sh
# Оставляет N последних версий образов приложения (по умолчанию 3). Базы слайсеров не трогает.
# Образ, на котором работает контейнер, Docker не удалит — это нормально.
set -eu
keep=${1:-3}
for repo in goat-packer-api goat-packer-worker goat-packer-gateway goat-packer-runner-bambu goat-packer-runner-orca; do
  docker image ls "$repo" --format '{{.CreatedAt}}|{{.Tag}}' | sort -r | awk -F'|' -v k="$keep" 'NR > k { print $2 }' |
    while read -r tag; do docker image rm "$repo:$tag" >/dev/null 2>&1 && echo "removed $repo:$tag" || true; done
done
```

- [ ] **Step 3: `.dockerignore` — дописать в конец**

```
deploy/vendor
docs
examples
```

- [ ] **Step 4: Проверить синтаксис и полную сборку версии на `main`**

```bash
chmod +x deploy/build-images.sh deploy/prune-images.sh
sh -n deploy/build-images.sh && sh -n deploy/prune-images.sh && echo sh-ok
git ls-files -co --exclude-standard | tar -cz -T - | ssh -o ControlPath=/tmp/main.sock main 'rm -rf ~/gp-build && mkdir ~/gp-build && tar -xz -C ~/gp-build'
ssh -o ControlPath=/tmp/main.sock main 'cd ~/gp-build && sh deploy/build-images.sh plan-test 2>&1 | tail -8'
```

Expected: `sh-ok`; в конце таблица из пяти образов `goat-packer-{runner-bambu,runner-orca,api,worker,gateway}:plan-test`. Первая сборка базы Orca занимает несколько минут, повторный запуск базы не пересобирает.

- [ ] **Step 5: Проверить очистку**

```bash
ssh -o ControlPath=/tmp/main.sock main 'cd ~/gp-build && for t in a b c d; do docker tag goat-packer-api:plan-test goat-packer-api:prune-$t; sleep 1; done
sh deploy/prune-images.sh 2; docker image ls goat-packer-api --format "{{.Tag}}"'
```

Expected: остаются два тега `goat-packer-api` (теги с одним ID создаются одновременно с исходным образом, поэтому скрипт может удалить любые два из одинаковых по времени — проверяем только, что их стало `2`). Затем убрать тестовые: `docker image ls --format "{{.Repository}}:{{.Tag}}" | grep -E ":(plan-test|prune-)" | xargs -r docker image rm`.

- [ ] **Step 6: Commit**

```bash
git add deploy/build-images.sh deploy/prune-images.sh .dockerignore
git commit -m "Single shell entry point to build and prune goat-packer images"
```

### Task 4: compose без сборки

**Files:**
- Modify: `deploy/compose.yml`

- [ ] **Step 1: Сервисы `api`, `worker`, `gateway` — образ вместо `build`**

В `deploy/compose.yml` заменить:

```yaml
  api:
    <<: *runtime
    build: {context: .., dockerfile: deploy/Dockerfile.backend, target: api}
```

на

```yaml
  api:
    <<: *runtime
    # Образы собирает CI прямо в Docker этой машины (deploy/build-images.sh), реестра нет.
    image: goat-packer-api:${GOAT_VERSION:?set GOAT_VERSION (e.g. v0.1.0)}
    pull_policy: never
```

`worker`: `build: {context: .., dockerfile: deploy/Dockerfile.backend, target: worker}` → 

```yaml
    image: goat-packer-worker:${GOAT_VERSION:?set GOAT_VERSION (e.g. v0.1.0)}
    pull_policy: never
```

`gateway`: `build: {context: .., dockerfile: deploy/Dockerfile.frontend}` →

```yaml
    image: goat-packer-gateway:${GOAT_VERSION:?set GOAT_VERSION (e.g. v0.1.0)}
    pull_policy: never
```

- [ ] **Step 2: Образы runner — той же версии**

В `worker.environment` заменить:

```yaml
      RUNNER_BAMBU_IMAGE: ${RUNNER_BAMBU_IMAGE:-goat-runner-bambu:2.8.2.61}
      RUNNER_ORCA_IMAGE: ${RUNNER_ORCA_IMAGE:-goat-runner-orca:2.4.2}
```

на

```yaml
      # Runner той же версии, что и приложение: в нём бинарники упаковщика этого релиза.
      RUNNER_BAMBU_IMAGE: goat-packer-runner-bambu:${GOAT_VERSION}
      RUNNER_ORCA_IMAGE: goat-packer-runner-orca:${GOAT_VERSION}
```

- [ ] **Step 3: Удалить сервисы `runner-bambu` и `runner-orca`** (блоки с `profiles: [images]` целиком, от `  runner-bambu:` до строки перед `volumes:` верхнего уровня).

- [ ] **Step 4: Проверить**

```bash
GOAT_VERSION=v0.0.0 DATABASE_PASSWORD=x S3_SECRET_KEY=x ADMIN_PASSWORD=x docker compose -f deploy/compose.yml config -q && echo compose-ok
docker compose -f deploy/compose.yml config -q 2>&1 | grep -q GOAT_VERSION && echo "requires GOAT_VERSION"
grep -n -E "build:|runner-(bambu|orca):|goat-runner" deploy/compose.yml || echo "no builds left"
```

Expected: `compose-ok`, `requires GOAT_VERSION`, `no builds left`.

- [ ] **Step 5: Commit**

```bash
git add deploy/compose.yml
git commit -m "Compose runs prebuilt goat-packer images of GOAT_VERSION instead of building"
```

### Task 5: Убрать python и старую установку

**Files:**
- Delete: `deploy/configure.py`, `deploy/fetch-runners.py`, `deploy/runner/extract-appimage.py`, `deploy/start.sh`, `deploy/runners.lock.json`
- Modify: `.gitignore`

- [ ] **Step 1: Удалить файлы**

```bash
git rm -q deploy/configure.py deploy/fetch-runners.py deploy/runner/extract-appimage.py deploy/start.sh deploy/runners.lock.json
grep -rn -E "configure\.py|fetch-runners|extract-appimage|start\.sh|runners\.lock" --include='*.md' --include='*.yml' --include='*.yaml' --include='*.kt' --include='*.sh' --include='Dockerfile*' . | grep -v '^./docs/superpowers/' || echo "no references"
```

Expected: ссылки остаются только в `deploy/README.md` (переписывается в задаче 8) — других нет.

- [ ] **Step 2: `.gitignore` — убрать устаревшие строки**

Удалить блоки:

```
# Private deployment config and local scratch
deploy/.env
deploy/sizing.json

# Verified upstream binaries (downloaded at build time, not Git)
deploy/vendor/
```

и вместо них оставить одну строку (локальный env для ручного `docker compose`):

```
deploy/.env
```

- [ ] **Step 3: Удалить локальные артефакты старой установки**

```bash
rm -rf deploy/vendor deploy/__pycache__ deploy/runner/__pycache__
ls deploy deploy/runner
```

Expected: в `deploy/` — `Dockerfile.backend`, `Dockerfile.frontend`, `Dockerfile.runner`, `Dockerfile.slicer`, `README.md`, `build-images.sh`, `compose.yml`, `nginx.conf`, `prune-images.sh`, `slicers.env`, `smoke.py`, `.env` (локальный, игнорируется); в `deploy/runner/` — `slicer.sh`.

- [ ] **Step 4: Commit**

```bash
git add .gitignore
git commit -m "Remove python setup scripts and server-side image build"
```

### Task 6: `.woodpecker/check.yaml`

**Files:**
- Create: `.woodpecker/check.yaml`

- [ ] **Step 1: Файл**

```yaml
# Проверки goat_packer на main: Rust, backend (Testcontainers через docker socket хоста), frontend.
# Тег тоже проверяется: release.yaml ждёт успешного check.
when:
  - event: [push, pull_request, manual, tag]

labels:
  platform: linux/amd64
  host: main

steps:
  - name: rust
    image: rust:1.94.0-bookworm
    volumes:
      - /var/lib/ci/goat_packer/cargo-registry:/usr/local/cargo/registry
      - /var/lib/ci/goat_packer/target:/woodpecker/src/github.com/petrpopov/goat_packer/target
    environment:
      CARGO_BUILD_JOBS: "3"
    commands:
      - cargo test --release --all --locked

  - name: backend
    image: eclipse-temurin:21-jdk-jammy
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock
      - /var/lib/ci/goat_packer/gradle:/root/.gradle
    commands:
      - cd webapp/backend && ./gradlew --no-daemon test

  - name: frontend
    image: oven/bun:1.3.14
    commands:
      - cd webapp/frontend
      - bun install --frozen-lockfile
      - bun test tests
      - bun run typecheck
      - bun run lint
      - bun run build

  # Кэш сборки Rust растёт с каждой версией зависимостей: чистим, если больше 10 ГБ (диск main 50 ГБ).
  - name: cleanup target cache
    image: rust:1.94.0-bookworm
    volumes:
      - /var/lib/ci/goat_packer/target:/woodpecker/src/github.com/petrpopov/goat_packer/target
    when:
      - status: [success, failure]
    commands:
      - 'size=$(du -sm target | cut -f1); echo "кэш target: $size МБ"; if [ "$size" -gt 10000 ]; then find target -mindepth 1 -delete; echo "кэш очищен"; fi'
```

- [ ] **Step 2: Проверить YAML**

```bash
python3 -c "import yaml,sys; yaml.safe_load(open('.woodpecker/check.yaml')); print('yaml-ok')" 2>/dev/null || ruby -ryaml -e 'YAML.load_file(".woodpecker/check.yaml"); puts "yaml-ok"'
```

Expected: `yaml-ok` (это только проверка синтаксиса локально; сам файл python не использует).

- [ ] **Step 3: Commit**

```bash
git add .woodpecker/check.yaml
git commit -m "Woodpecker check pipeline: Rust, backend with Testcontainers, frontend"
```

### Task 7: `.woodpecker/release.yaml`

**Files:**
- Create: `.woodpecker/release.yaml`

- [ ] **Step 1: Файл**

```yaml
# Выпуск по метке v*: образы goat-packer-*:<метка> прямо в Docker на main (реестра нет — их запускает
# Dokploy на этой же машине), архив CLI под Linux x86_64 и черновик GitHub Release. Ждёт успешного check.
when:
  - event: tag
    ref: refs/tags/v*

depends_on:
  - check

labels:
  platform: linux/amd64
  host: main

steps:
  - name: images
    image: docker:29-cli
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock
    commands:
      - sh deploy/build-images.sh "$CI_COMMIT_TAG"

  - name: cli archive
    image: rust:1.94.0-bookworm
    volumes:
      - /var/lib/ci/goat_packer/cargo-registry:/usr/local/cargo/registry
      - /var/lib/ci/goat_packer/target:/woodpecker/src/github.com/petrpopov/goat_packer/target
    environment:
      CARGO_BUILD_JOBS: "3"
    commands:
      - cargo build --release --locked --bins
      - mkdir -p dist
      - tar czf "dist/goat-packer-$CI_COMMIT_TAG-linux-x86_64.tar.gz" -C target/release colored_3mf_packer daily_scheduler verify_outputs

  - name: github release (draft)
    image: alpine:3.22
    environment:
      GH_TOKEN:
        from_secret: github_token
    commands:
      - apk add -q github-cli
      - gh release view "$CI_COMMIT_TAG" -R "$CI_REPO" >/dev/null 2>&1 || gh release create "$CI_COMMIT_TAG" -R "$CI_REPO" --draft --title "$CI_COMMIT_TAG" --generate-notes
      - gh release upload "$CI_COMMIT_TAG" -R "$CI_REPO" --clobber dist/goat-packer-*.tar.gz

  - name: prune old images
    image: docker:29-cli
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock
    commands:
      - sh deploy/prune-images.sh 3
```

- [ ] **Step 2: Проверить YAML** (как в задаче 6, для `.woodpecker/release.yaml`). Expected: `yaml-ok`.

- [ ] **Step 3: Commit**

```bash
git add .woodpecker/release.yaml
git commit -m "Woodpecker release pipeline: local images, CLI archive, draft GitHub release"
```

### Task 8: README сборки

**Files:**
- Modify: `deploy/README.md`
- Modify: `CLAUDE.md` (раздел Commands)

- [ ] **Step 1: В `deploy/README.md` заменить заголовок и разделы «Новый сервер» и «Обновление существующего запуска»** (от `# Развёртывание по IP…` до строки перед `## Ресурсы и исполнение`) на:

```markdown
# Сборка и развёртывание

Образы собирает Woodpecker (`ci.ezheg.xyz`) прямо в Docker на `main`; реестра нет.

- Push в любую ветку: `.woodpecker/check.yaml` — тесты Rust, backend (Testcontainers), frontend.
- Тег `v*`: `.woodpecker/release.yaml` — `deploy/build-images.sh <тег>` собирает
  `goat-packer-{api,worker,gateway,runner-bambu,runner-orca}:<тег>`, черновик GitHub Release получает
  `goat-packer-<тег>-linux-x86_64.tar.gz` (CLI), хранятся образы трёх последних версий.

Базы слайсеров `goat-packer-slicer-{bambu,orca}:<версия>` описаны в `deploy/slicers.env` (URL, SHA256,
offset SquashFS) и собираются один раз; AppImage при сборке не запускается. Смена версии слайсера —
правка `slicers.env`, следующий релиз соберёт новую базу.

Руками на x86-64 машине с Docker: `sh deploy/build-images.sh dev`, затем
`GOAT_VERSION=dev docker compose --env-file deploy/.env -f deploy/compose.yml up -d`.
Развёртывание на сервер — через Dokploy (см. ниже, раздел добавляется в части 3).
```

- [ ] **Step 2: Убрать оставшиеся упоминания старой установки**

```bash
grep -n -E "configure\.py|start\.sh|fetch-runners|--update|--public-ip" deploy/README.md
```

Каждую найденную строку переписать под новую схему: ресурсы задаются переменными окружения (`MAX_PARALLEL`, `EXECUTION_CPU_BUDGET`, `EXECUTION_MEMORY_MIB`, `WORKER_CPUS`, `WORKER_MEMORY_MIB`, `STORAGE_LIMIT_BYTES`) в Dokploy или `deploy/.env`. Повторный `grep` ничего не находит.

- [ ] **Step 3: `CLAUDE.md`, раздел `## Commands` — дописать после блока команд**

```markdown
CI: Woodpecker `ci.ezheg.xyz` (`.woodpecker/`): `check` на каждый push, `release` по тегу `v*` собирает образы
`goat-packer-*:<тег>` в Docker на `main` через `deploy/build-images.sh` (единственное описание сборки образов).
```

- [ ] **Step 4: Commit**

```bash
git add deploy/README.md CLAUDE.md
git commit -m "Document CI image build; drop configure/start instructions"
```

### Task 9: Подключить репозиторий к Woodpecker и прогнать check (владелец + «да» на push)

- [ ] **Step 1: Владелец в `https://ci.ezheg.xyz`**

1. Add repository → `petrpopov/goat_packer` → Enable.
2. Settings → Project → Trusted: включить **Volumes** (host-пути кэшей и `/var/run/docker.sock`).
3. Settings → Secrets → Add: имя `github_token`, значение — GitHub token с правом `contents: write` на `petrpopov/goat_packer` (fine-grained) или classic `repo`; events: `tag`.

- [ ] **Step 2: Каталоги кэшей на `main`**

```bash
ssh -o ControlPath=/tmp/main.sock main 'sudo mkdir -p /var/lib/ci/goat_packer/{cargo-registry,target,gradle} && ls /var/lib/ci/goat_packer'
```

- [ ] **Step 3: Push ветки (нужно «да»)**

```bash
git push -u origin production-deploy
```

- [ ] **Step 4: Дождаться `check`**

```bash
/tmp/wpq.sh "select p.number, p.status, p.branch, s.name, s.state, (s.finished-s.started)/60 mins from pipelines p join repos r on r.id=p.repo_id join steps s on s.pipeline_id=p.id where r.full_name='petrpopov/goat_packer' and p.id=(select max(p2.id) from pipelines p2 where p2.repo_id=r.id) order by s.pid;"
```

(`/tmp/wpq.sh` — read-only запрос к базе Woodpecker, см. часть 1; либо смотреть в UI.)

Expected: шаги `rust`, `backend`, `frontend`, `cleanup target cache` — `success`.

Если `backend` падает на подключении к контейнерам Testcontainers (timeout к `localhost:<порт>` или к IP шлюза): шаг работает в отдельной сети пайплайна, а контейнеры тестов публикуют порты на хосте `main`. Исправление — в шаг `backend` добавить

```yaml
    extra_hosts:
      - host.docker.internal:host-gateway
    environment:
      TESTCONTAINERS_HOST_OVERRIDE: host.docker.internal
```

и комментарий над ним с причиной. Если ошибка другая — читать лог шага, не угадывать.

### Task 10: Первый выпуск `v0.1.0` (нужно «да»)

- [ ] **Step 1: Слить ветку в `master` и отправить** (после зелёного check)

```bash
git switch master && git merge --ff-only production-deploy && git push origin master
```

Если `--ff-only` невозможен (на GitHub `master` ушёл вперёд) — остановиться и спросить владельца.

- [ ] **Step 2: Тег**

```bash
git tag -a v0.1.0 -m "v0.1.0: server mode, CI-built images" && git push origin v0.1.0
```

- [ ] **Step 3: Проверить результат**

```bash
ssh -o ControlPath=/tmp/main.sock main 'docker image ls --filter "reference=goat-packer-*" --format "{{.Repository}}:{{.Tag}} {{.Size}}"'
gh release view v0.1.0 -R petrpopov/goat_packer --json isDraft,assets --jq '{draft:.isDraft, assets:[.assets[].name]}'
```

Expected: пять образов `:v0.1.0` и две базы `goat-packer-slicer-*`; `{"draft":true,"assets":["goat-packer-v0.1.0-linux-x86_64.tar.gz"]}`.

- [ ] **Step 4: Убрать временный каталог сборки на `main`**

```bash
ssh -o ControlPath=/tmp/main.sock main 'rm -rf ~/gp-build'
```

---

## Критерии готовности части 2

1. Push в ветку запускает `check` на `main-linux`; все шаги зелёные.
2. Тег `v0.1.0`: в Docker на `main` пять образов `goat-packer-*:v0.1.0`, базы слайсеров собраны один раз.
3. Черновик GitHub Release `v0.1.0` с архивом CLI.
4. В репозитории нет python-скриптов установки; `deploy/compose.yml` ничего не собирает и требует `GOAT_VERSION`.
5. Старый тестовый стенд `~/goat_packer` на `main` пока работает (убирается в части 3).
