# Часть 1. Общий CI и один Docker на mbp15 — план реализации

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Woodpecker `kdr-ci` становится общим CI `ci` для всех проектов, на `mbp15` остаётся один colima-профиль `default` для Dokploy и сборок, туннель CI чинится и пробрасывает Dokploy API на `main`.

**Architecture:** Спецификация: `docs/superpowers/specs/2026-10-15-ci-dokploy-deploy-design.md`, часть 1. Это операции с живой инфраструктурой, а не код: каждая задача заканчивается проверкой состояния. Конфигурация хранится в `hlab-infra/ci/`. Секреты не копируются в git и не печатаются в вывод.

**Tech Stack:** Woodpecker 3.19.0 (docker и local backend), Docker Compose, colima 0.10.3 (MacPorts) на macOS, launchd, OpenSSH, ufw.

---

## Факты на 2026-10-15

- `main` (`ssh main`, Ubuntu 24.04, user `petrpopov`, sudo без пароля): стек `~/kdr-ci` (compose-проект `kdr-ci`: `woodpecker-server` v3.19.0 на `127.0.0.1:8000/9000`, `woodpecker-agent` `main-linux`, `newt` в host network). Volumes `kdr-ci_woodpecker-data`, `kdr-ci_woodpecker-agent`. Секреты в `~/kdr-ci/.env` (ключи `WOODPECKER_GITHUB_CLIENT`, `WOODPECKER_GITHUB_SECRET`, `WOODPECKER_AGENT_SECRET`, `PANGOLIN_ENDPOINT`, `NEWT_ID`, `NEWT_SECRET`). Кэши `kdr`: `/var/lib/kdr-ci/{mirror.git,cargo-registry,target-linux}`.
- ufw на `main`: снаружи только 2783 с `LIMIT`. Для SSH использовать одно мультиплексированное соединение:
  `ssh -o ControlMaster=auto -o ControlPath=/tmp/main.sock -o ControlPersist=2h main ...`.
  Никаких циклов, открывающих новое соединение каждые несколько секунд.
- `mbp15` (`ssh mbp15`, user `devtest`, Intel, macOS 12): запущен colima профиля `default` (MacPorts `/opt/local/bin/colima` 0.10.3; 4 CPU, 10 GB, 60 GB, qemu), сокет `/Users/devtest/.colima/default/docker.sock`. На нём Dokploy (`:3000`), fancyforia, dozzle, beszel-agent.
- launchd на `mbp15`: `/Library/LaunchDaemons/com.ezheg.colima.plist` (запускает zerobrew `/opt/zerobrew/bin/colima start`), `/Library/LaunchDaemons/com.ezheg.kdr-ci-agent.plist` (local-агент `mbp15-mac`, env `~/kdr-ci/agent.env`), `/Library/LaunchDaemons/com.ezheg.kdr-ci-tunnel.plist` (`ssh -N -L 19000:127.0.0.1:9000 -p 2783 petrpopov@195.208.61.28`, ключ `~/.ssh/kdr_ci_tunnel`), `/Library/LaunchDaemons/com.ezheg.newt.plist` (Pangolin, не трогать); `~/Library/LaunchAgents/com.ezheg.colima.plist` (MacPorts colima `start --cpu 2 ...`, падает с кодом 1); задание `com.devtest.colima` без файла.
- `~/kdr-ci/agent-docker.env` описывает агента `mbp15-docker` с `DOCKER_HOST=.../.colima/kdr-ci/docker.sock`; профиля `kdr-ci` нет, агент не запущен.
- `kdr` (`../kdr`, ветка `master`): Linux-пайплайны `.woodpecker/check.yaml`, `release.yaml` используют `/var/lib/kdr-ci/*` и `labels: platform: linux/amd64`; Mac-пайплайны `check-mac.yaml`, `release-mac.yaml` используют `/Users/devtest/kdr-ci/{cache/target,bin}`. Push-адреса `origin`: `ssh://main/var/lib/kdr-ci/mirror.git` и GitHub.
- `hlab-infra` (`../hlab-infra`, ветка `master`): в рабочей копии чужие незакоммиченные изменения (`CLAUDE.md`, `AGENTS.md`, `nl_pass.txt`) — не трогать, коммитить только свои файлы по именам, никогда `git add -A`.

## Точки согласования с владельцем

Перед задачами 4, 6 и 8 (простой CI) и задачей 7 (простой сервисов hlab на ~1 минуту) получить явное «да» в чате. Перед `git push` в `kdr` и `hlab-infra` — тоже.

---

### Task 1: Резервные копии и снимок состояния

**Files:** только каталоги резервных копий на хостах.

- [ ] **Step 1: Снимок `main`**

```bash
ssh -o ControlMaster=auto -o ControlPath=/tmp/main.sock -o ControlPersist=2h main '
set -e; b=~/backup-ci-2026-10-15; mkdir -p $b
cp -a ~/kdr-ci $b/kdr-ci-dir
docker ps -a --format "{{.Names}} {{.Status}}" > $b/docker-ps.txt
for v in kdr-ci_woodpecker-data kdr-ci_woodpecker-agent; do
  docker run --rm -v $v:/v:ro -v $b:/b alpine tar czf /b/$v.tgz -C /v .
done
ls -la $b'
```

Expected: два архива `kdr-ci_woodpecker-*.tgz` ненулевого размера и копия `kdr-ci-dir`.

- [ ] **Step 2: Снимок `mbp15`**

```bash
ssh mbp15 '
set -e; b=~/backup-ci-2026-10-15; mkdir -p $b
cp /Library/LaunchDaemons/com.ezheg.colima.plist /Library/LaunchDaemons/com.ezheg.kdr-ci-*.plist $b/
cp ~/Library/LaunchAgents/com.ezheg.colima.plist $b/LaunchAgent-com.ezheg.colima.plist
cp -a ~/kdr-ci/agent.env ~/kdr-ci/agent-docker.env ~/kdr-ci/agent.conf $b/
cp ~/.colima/default/colima.yaml $b/
ls $b'
```

Expected: в списке 3 LaunchDaemon-файла, LaunchAgent, два env, `agent.conf`, `colima.yaml`.

### Task 2: Конфигурация `ci` в `hlab-infra`

**Files:**
- Create: `../hlab-infra/ci/docker-compose.yml`
- Create: `../hlab-infra/ci/.env.example`
- Create: `../hlab-infra/ci/mbp15/agent-mac.env`, `../hlab-infra/ci/mbp15/agent-docker.env`
- Create: `../hlab-infra/ci/mbp15/com.ezheg.ci-agent.plist`, `com.ezheg.ci-agent-docker.plist`, `com.ezheg.ci-tunnel.plist`, `com.ezheg.colima.plist`
- Create: `../hlab-infra/ci/mbp15/colima-up.sh`
- Create: `../hlab-infra/ci/README.md`

- [ ] **Step 1: compose для `main`**

`../hlab-infra/ci/docker-compose.yml`:

```yaml
# Общий CI всех проектов: Woodpecker (сервер + исполнитель main-linux) и newt (Pangolin: ci.ezheg.xyz -> 127.0.0.1:8000).
# Работает на main в ~/ci (compose-проект ci), секреты в ~/ci/.env (образец .env.example). Не Dokploy:
# CI не должен зависеть от Dokploy на mbp15. Агенты mbp15 ходят к gRPC 9000 через ssh-туннель (mbp15/).
services:
  newt:
    image: fosrl/newt:latest
    restart: unless-stopped
    environment:
      PANGOLIN_ENDPOINT: ${PANGOLIN_ENDPOINT}
      NEWT_ID: ${NEWT_ID}
      NEWT_SECRET: ${NEWT_SECRET}
    network_mode: host

  woodpecker-server:
    image: woodpeckerci/woodpecker-server:v3.19.0
    restart: unless-stopped
    ports:
      - 127.0.0.1:8000:8000   # веб и уведомления GitHub, наружу через Pangolin
      - 127.0.0.1:9000:9000   # gRPC исполнителей (mbp15 ходит ssh-туннелем)
    volumes:
      - woodpecker-data:/var/lib/woodpecker
    environment:
      WOODPECKER_HOST: https://ci.ezheg.xyz
      WOODPECKER_LOG_LEVEL: info
      WOODPECKER_OPEN: "false"
      WOODPECKER_ADMIN: petrpopov
      WOODPECKER_GITHUB: "true"
      WOODPECKER_GITHUB_CLIENT: ${WOODPECKER_GITHUB_CLIENT}
      WOODPECKER_GITHUB_SECRET: ${WOODPECKER_GITHUB_SECRET}
      WOODPECKER_AGENT_SECRET: ${WOODPECKER_AGENT_SECRET}
      # Проверки kdr на Mac идут дольше часа.
      WOODPECKER_MAX_PIPELINE_TIMEOUT: "360"

  woodpecker-agent:
    image: woodpeckerci/woodpecker-agent:v3.19.0
    restart: unless-stopped
    depends_on: [woodpecker-server]
    volumes:
      - woodpecker-agent:/etc/woodpecker
      - /var/run/docker.sock:/var/run/docker.sock
    environment:
      WOODPECKER_SERVER: woodpecker-server:9000
      WOODPECKER_AGENT_SECRET: ${WOODPECKER_AGENT_SECRET}
      WOODPECKER_HOSTNAME: main-linux
      # Пайплайны выбирают машину меткой host; platform у main и mbp15-docker одинаковый.
      WOODPECKER_AGENT_LABELS: host=main
      WOODPECKER_MAX_WORKFLOWS: "1"

volumes:
  woodpecker-data:
  woodpecker-agent:
```

- [ ] **Step 2: образец секретов**

`../hlab-infra/ci/.env.example`:

```dotenv
# Копия в ~/ci/.env на main (chmod 600). Значения переносятся из прежнего ~/kdr-ci/.env.
WOODPECKER_GITHUB_CLIENT=
WOODPECKER_GITHUB_SECRET=
WOODPECKER_AGENT_SECRET=
PANGOLIN_ENDPOINT=https://pangolin.ezheg.xyz
NEWT_ID=
NEWT_SECRET=
```

- [ ] **Step 3: env агентов `mbp15` (без секрета; секрет дописывается на хосте)**

`../hlab-infra/ci/mbp15/agent-mac.env`:

```sh
# Агент mbp15-mac (backend local): сборки под macOS. Секрет WOODPECKER_AGENT_SECRET дописывается на хосте.
WOODPECKER_SERVER=127.0.0.1:19000
WOODPECKER_BACKEND=local
WOODPECKER_HOSTNAME=mbp15-mac
WOODPECKER_AGENT_LABELS=host=mbp15
WOODPECKER_MAX_WORKFLOWS=1
WOODPECKER_BACKEND_LOCAL_TEMP_DIR=/Users/devtest/ci/work
WOODPECKER_AGENT_CONFIG_FILE=/Users/devtest/ci/agent-mac.conf
PATH=/Users/devtest/ci/bin:/Users/devtest/.cargo/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin
```

`../hlab-infra/ci/mbp15/agent-docker.env`:

```sh
# Агент mbp15-docker (backend docker) в том же Docker colima default, что и Dokploy.
WOODPECKER_SERVER=127.0.0.1:19000
WOODPECKER_BACKEND=docker
WOODPECKER_HOSTNAME=mbp15-docker
WOODPECKER_AGENT_LABELS=host=mbp15
WOODPECKER_MAX_WORKFLOWS=1
WOODPECKER_AGENT_CONFIG_FILE=/Users/devtest/ci/agent-docker.conf
DOCKER_HOST=unix:///Users/devtest/.colima/default/docker.sock
```

- [ ] **Step 4: launchd-задания**

`../hlab-infra/ci/mbp15/com.ezheg.ci-agent.plist`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>com.ezheg.ci-agent</string>
  <key>UserName</key><string>devtest</string>
  <key>EnvironmentVariables</key><dict><key>HOME</key><string>/Users/devtest</string></dict>
  <key>ProgramArguments</key>
  <array>
    <string>/bin/sh</string><string>-c</string>
    <string>set -a; . /Users/devtest/ci/agent-mac.env; exec /Users/devtest/ci/bin/woodpecker-agent</string>
  </array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>ThrottleInterval</key><integer>10</integer>
  <key>StandardOutPath</key><string>/Users/devtest/ci/agent-mac.log</string>
  <key>StandardErrorPath</key><string>/Users/devtest/ci/agent-mac.log</string>
</dict>
</plist>
```

`../hlab-infra/ci/mbp15/com.ezheg.ci-agent-docker.plist` — тот же файл с отличиями: `Label` = `com.ezheg.ci-agent-docker`, команда `set -a; . /Users/devtest/ci/agent-docker.env; exec /Users/devtest/ci/bin/woodpecker-agent`, лог `/Users/devtest/ci/agent-docker.log`. Записать файл целиком:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>com.ezheg.ci-agent-docker</string>
  <key>UserName</key><string>devtest</string>
  <key>EnvironmentVariables</key><dict><key>HOME</key><string>/Users/devtest</string></dict>
  <key>ProgramArguments</key>
  <array>
    <string>/bin/sh</string><string>-c</string>
    <string>set -a; . /Users/devtest/ci/agent-docker.env; exec /Users/devtest/ci/bin/woodpecker-agent</string>
  </array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>ThrottleInterval</key><integer>10</integer>
  <key>StandardOutPath</key><string>/Users/devtest/ci/agent-docker.log</string>
  <key>StandardErrorPath</key><string>/Users/devtest/ci/agent-docker.log</string>
</dict>
</plist>
```

`../hlab-infra/ci/mbp15/com.ezheg.ci-tunnel.plist`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>com.ezheg.ci-tunnel</string>
  <key>UserName</key><string>devtest</string>
  <!-- -L: gRPC Woodpecker для агентов mbp15. -R: Dokploy API для шага деплоя на main (только 127.0.0.1 main). -->
  <key>ProgramArguments</key>
  <array>
    <string>/usr/bin/ssh</string><string>-N</string>
    <string>-i</string><string>/Users/devtest/.ssh/kdr_ci_tunnel</string>
    <string>-p</string><string>2783</string>
    <string>-o</string><string>BatchMode=yes</string>
    <string>-o</string><string>ExitOnForwardFailure=yes</string>
    <string>-o</string><string>ServerAliveInterval=30</string>
    <string>-o</string><string>ServerAliveCountMax=3</string>
    <string>-L</string><string>127.0.0.1:19000:127.0.0.1:9000</string>
    <string>-R</string><string>127.0.0.1:13000:127.0.0.1:3000</string>
    <string>petrpopov@195.208.61.28</string>
  </array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <!-- ufw LIMIT на main режет частые подключения: не переподключаться чаще раза в 30 с. -->
  <key>ThrottleInterval</key><integer>30</integer>
  <key>StandardErrorPath</key><string>/Users/devtest/ci/tunnel.log</string>
</dict>
</plist>
```

- [ ] **Step 5: единый запуск colima**

`../hlab-infra/ci/mbp15/colima-up.sh`:

```sh
#!/bin/sh
# Поднимает единственную ВМ colima (профиль default): Docker для Dokploy, сервисов hlab и агента mbp15-docker.
# Зовётся LaunchDaemon com.ezheg.colima при загрузке. Параметры ВМ — в ~/.colima/default/colima.yaml.
# Повторяет попытки: сразу после загрузки сеть и lima могут быть не готовы.
export PATH=/opt/local/bin:/usr/bin:/bin:/usr/sbin:/sbin HOME=/Users/devtest
for i in 1 2 3 4 5 6 7 8 9 10; do
  if colima status -p default >/dev/null 2>&1; then echo "$(date) default уже запущена"; exit 0; fi
  echo "$(date) запуск default, попытка $i"
  colima start -p default && exit 0
  # Зависший после сбоя экземпляр мешает следующему запуску.
  colima stop -p default --force >/dev/null 2>&1
  sleep 30
done
exit 1
```

`../hlab-infra/ci/mbp15/com.ezheg.colima.plist`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>com.ezheg.colima</string>
  <key>UserName</key><string>devtest</string>
  <key>WorkingDirectory</key><string>/Users/devtest</string>
  <key>ProgramArguments</key>
  <array><string>/bin/sh</string><string>/Users/devtest/ci/bin/colima-up.sh</string></array>
  <key>RunAtLoad</key><true/>
  <key>StandardOutPath</key><string>/Users/devtest/Library/Logs/colima.out.log</string>
  <key>StandardErrorPath</key><string>/Users/devtest/Library/Logs/colima.err.log</string>
</dict>
</plist>
```

- [ ] **Step 6: README**

`../hlab-infra/ci/README.md`:

````markdown
# ci — общий Woodpecker для всех проектов

`https://ci.ezheg.xyz` (Pangolin, SSO). Сервер и агент `main-linux` — на `main` в `~/ci`
(`docker-compose.yml` отсюда, секреты `~/ci/.env` по образцу `.env.example`). Не Dokploy: CI не зависит от `mbp15`.

| Агент | Где | Backend | Метки |
|---|---|---|---|
| `main-linux` | main | docker (`/var/run/docker.sock` main) | `platform=linux/amd64`, `host=main` |
| `mbp15-mac` | mbp15 | local (`~/ci/work`) | `platform=darwin/amd64`, `host=mbp15` |
| `mbp15-docker` | mbp15 | docker (colima `default`, тот же, что у Dokploy) | `platform=linux/amd64`, `host=mbp15` |

`platform` у `main-linux` и `mbp15-docker` совпадает: пайплайн, которому нужна конкретная машина
(кэши, локальные образы), обязан указать `labels: host: main` или `host: mbp15`.

## mbp15

Файлы в `mbp15/`: env агентов (секрет `WOODPECKER_AGENT_SECRET` дописывается на хосте), LaunchDaemons
`com.ezheg.ci-agent`, `com.ezheg.ci-agent-docker`, `com.ezheg.ci-tunnel`, `com.ezheg.colima` и `colima-up.sh`.
Один colima (MacPorts `/opt/local/bin/colima`), один профиль `default`.

Туннель `com.ezheg.ci-tunnel` (ключ `~/.ssh/kdr_ci_tunnel`): `-L 127.0.0.1:19000 → main:9000` (gRPC агентов),
`-R main:127.0.0.1:13000 → mbp15:3000` (Dokploy API для шагов деплоя). На `main` ufw `LIMIT` на SSH:
частые переподключения режутся, поэтому `ThrottleInterval` 30 с.

## Кэши на main

`/var/lib/ci/<проект>/` (у `kdr`: `mirror.git`, `cargo-registry`, `target-linux`). Volumes в пайплайнах
требуют trusted-репозитория (Settings → Project → Trusted → Volumes).

## Новый проект

1. Открыть `ci.ezheg.xyz` → Add repository → включить.
2. Если нужны volumes или docker socket: Settings → Trusted (volumes / security).
3. Секреты проекта — Settings → Secrets (например `github_token` для выпусков).
4. В `.woodpecker/*.yaml` указать `labels: platform: …` и `host: …`.
````

- [ ] **Step 7: Проверить синтаксис и закоммитить в `hlab-infra`**

```bash
cd ../hlab-infra
docker compose -f ci/docker-compose.yml --env-file ci/.env.example config -q && echo compose-ok
for f in ci/mbp15/*.plist; do plutil -lint "$f"; done
sh -n ci/mbp15/colima-up.sh && echo sh-ok
git add ci/docker-compose.yml ci/.env.example ci/README.md ci/mbp15/
git commit -m "ci: shared Woodpecker stack for all projects (was kdr-ci), mbp15 agents/tunnel/colima"
```

Expected: `compose-ok`, каждый plist `OK`, `sh-ok`, коммит создан. Не пушить до задачи 9.

### Task 3: Пайплайны `kdr` — метки хоста и новые пути (до запуска нового стека)

Сначала метки, потом docker-агент на `mbp15`: иначе Linux-сборки `kdr` могут уйти на `mbp15-docker`, где нет их кэшей и образа `kdr-ci-linux:1`.

**Files:**
- Modify: `../kdr/.woodpecker/check.yaml`, `../kdr/.woodpecker/release.yaml` (`labels`, `/var/lib/kdr-ci` → `/var/lib/ci/kdr`, комментарии про копию)
- Modify: `../kdr/.woodpecker/check-mac.yaml`, `../kdr/.woodpecker/release-mac.yaml` (`labels`, `/Users/devtest/kdr-ci` → `/Users/devtest/ci`)
- Modify: `../kdr/docs/DEVELOPMENT.md` (раздел «Сборки и выпуски»: путь копии)

- [ ] **Step 1: Заменить пути и добавить метки**

```bash
cd ../kdr
sed -i '' 's#/var/lib/kdr-ci/#/var/lib/ci/kdr/#g' .woodpecker/check.yaml .woodpecker/release.yaml docs/DEVELOPMENT.md
sed -i '' 's#/Users/devtest/kdr-ci/#/Users/devtest/ci/#g' .woodpecker/check-mac.yaml .woodpecker/release-mac.yaml
for f in .woodpecker/check.yaml .woodpecker/release.yaml; do
  sed -i '' 's#^  platform: linux/amd64$#  platform: linux/amd64\n  host: main#' "$f"; done
for f in .woodpecker/check-mac.yaml .woodpecker/release-mac.yaml; do
  sed -i '' 's#^  platform: darwin/amd64$#  platform: darwin/amd64\n  host: mbp15#' "$f"; done
grep -n -A1 'platform:' .woodpecker/*.yaml; grep -rn 'kdr-ci' .woodpecker docs/DEVELOPMENT.md
```

Expected: под каждым `platform:` строка `host: main` или `host: mbp15` (если BSD sed вставил буквальное `\n`, поправить вручную через Edit). `grep kdr-ci` находит только имя образа `kdr-ci-linux:1` — его не трогаем, это образ сборщика `kdr`.

- [ ] **Step 2: Закоммитить, не пушить**

```bash
cd ../kdr && git add .woodpecker/check.yaml .woodpecker/release.yaml .woodpecker/check-mac.yaml .woodpecker/release-mac.yaml docs/DEVELOPMENT.md
git commit -m "ci: shared ci paths (/var/lib/ci/kdr, ~/ci) and host labels"
```

Пушить после задачи 6: пуш запустит сборку, она и будет проверкой нового стека.

### Task 4: Перенос стека на `main` (простой CI ~5 минут — нужно «да»)

- [ ] **Step 1: Остановить старый стек**

```bash
ssh -o ControlPath=/tmp/main.sock main 'cd ~/kdr-ci && docker compose ps --format "{{.Name}} {{.Status}}" && docker compose down'
```

Expected: `kdr-ci-*` контейнеры удалены, volumes остались (`docker volume ls | grep kdr-ci` — два).

- [ ] **Step 2: Скопировать volumes**

```bash
ssh -o ControlPath=/tmp/main.sock main '
set -e
for v in woodpecker-data woodpecker-agent; do
  docker volume create ci_$v >/dev/null
  docker run --rm -v kdr-ci_$v:/from:ro -v ci_$v:/to alpine sh -c "cp -a /from/. /to/"
  echo "$v: $(docker run --rm -v kdr-ci_$v:/v:ro alpine du -s /v | cut -f1) -> $(docker run --rm -v ci_$v:/v:ro alpine du -s /v | cut -f1)"
done'
```

Expected: размеры совпадают попарно.

- [ ] **Step 3: Перенести кэши `kdr`, оставить симлинк**

```bash
ssh -o ControlPath=/tmp/main.sock main '
set -e
sudo mkdir -p /var/lib/ci
sudo mv /var/lib/kdr-ci /var/lib/ci/kdr
sudo ln -s /var/lib/ci/kdr /var/lib/kdr-ci
ls -la /var/lib/ci /var/lib/ci/kdr; readlink /var/lib/kdr-ci'
```

Expected: `/var/lib/ci/kdr/{mirror.git,cargo-registry,target-linux}`, `/var/lib/kdr-ci -> /var/lib/ci/kdr`.

- [ ] **Step 4: Разложить конфигурацию и поднять `ci`**

```bash
scp -o ControlPath=/tmp/main.sock ../hlab-infra/ci/docker-compose.yml main:/tmp/ci-compose.yml
ssh -o ControlPath=/tmp/main.sock main '
set -e
mkdir -p ~/ci && chmod 700 ~/ci
mv /tmp/ci-compose.yml ~/ci/docker-compose.yml
install -m 600 ~/kdr-ci/.env ~/ci/.env
cd ~/ci && docker compose up -d && sleep 15 && docker compose ps --format "{{.Name}} {{.Status}}"
curl -s -o /dev/null -w "web %{http_code}\n" http://127.0.0.1:8000/
docker compose logs --since 1m woodpecker-agent | tail -5'
```

Expected: `ci-woodpecker-server-1`, `ci-woodpecker-agent-1`, `ci-newt-1` Up; `web 200`; в логе агента нет ошибок подключения.

- [ ] **Step 5: Проверить внешний адрес и данные**

```bash
curl -s -o /dev/null -w "%{http_code}\n" https://ci.ezheg.xyz/
```

Expected: 200 или 302 (SSO). Открыть `https://ci.ezheg.xyz` в браузере (владелец): репозиторий `kdr` и история сборок на месте; в Settings → Agents агент `main-linux` online с меткой `host=main`.

- [ ] **Step 6: Откат при неудаче**

Если шаги 4–5 не прошли: `cd ~/ci && docker compose down`; `sudo rm /var/lib/kdr-ci && sudo mv /var/lib/ci/kdr /var/lib/kdr-ci`; `cd ~/kdr-ci && docker compose up -d`. Volumes `kdr-ci_*` не тронуты.

### Task 5: Агенты и туннель на `mbp15` (простой Mac-сборок — нужно «да»)

- [ ] **Step 1: Каталог `~/ci` и файлы**

```bash
scp ../hlab-infra/ci/mbp15/agent-mac.env ../hlab-infra/ci/mbp15/agent-docker.env ../hlab-infra/ci/mbp15/colima-up.sh ../hlab-infra/ci/mbp15/*.plist mbp15:/tmp/
ssh mbp15 '
set -e
mkdir -p ~/ci/bin ~/ci/work ~/ci/cache
cp -a ~/kdr-ci/bin/woodpecker-agent ~/kdr-ci/bin/plugin-git ~/kdr-ci/bin/gh ~/ci/bin/
install -m 755 /tmp/colima-up.sh ~/ci/bin/colima-up.sh
# Кэш сборки kdr переносится целиком: экономит полную пересборку.
[ -d ~/kdr-ci/cache/target ] && mv ~/kdr-ci/cache/target ~/ci/cache/target
for n in mac docker; do
  install -m 600 /tmp/agent-$n.env ~/ci/agent-$n.env
  grep "^WOODPECKER_AGENT_SECRET=" ~/kdr-ci/agent.env >> ~/ci/agent-$n.env
done
# Mac-агент сохраняет свой id; docker-агент регистрируется заново.
cp ~/kdr-ci/agent.conf ~/ci/agent-mac.conf
grep -c AGENT_SECRET ~/ci/agent-*.env; ls ~/ci ~/ci/bin'
```

Expected: в каждом env ровно одна строка `AGENT_SECRET`; в `~/ci/bin` — `woodpecker-agent`, `plugin-git`, `gh`, `colima-up.sh`.

- [ ] **Step 2: Заменить launchd-задания агента и туннеля**

```bash
ssh mbp15 '
set -e
sudo launchctl bootout system/com.ezheg.kdr-ci-agent || true
sudo launchctl bootout system/com.ezheg.kdr-ci-tunnel || true
sudo mv /Library/LaunchDaemons/com.ezheg.kdr-ci-agent.plist /Library/LaunchDaemons/com.ezheg.kdr-ci-tunnel.plist ~/backup-ci-2026-10-15/ 2>/dev/null || true
for n in ci-tunnel ci-agent ci-agent-docker; do
  sudo install -o root -g wheel -m 644 /tmp/com.ezheg.$n.plist /Library/LaunchDaemons/
  sudo launchctl bootstrap system /Library/LaunchDaemons/com.ezheg.$n.plist
done
sleep 20; sudo launchctl list | grep -E "com.ezheg.ci-"; tail -3 ~/ci/tunnel.log; tail -3 ~/ci/agent-mac.log ~/ci/agent-docker.log'
```

Expected: три задания с PID; лог туннеля пустой или без ошибок; в логах агентов нет `waiting for server connection`. Если на `main` нет `-R` (ошибка `remote port forwarding failed`) — проверить `ssh -o ControlPath=/tmp/main.sock main 'sudo sshd -T | grep -E "allowtcpforwarding|gatewayports"'` (`allowtcpforwarding yes`).

- [ ] **Step 3: Проверить проброс Dokploy API на `main`**

```bash
ssh -o ControlPath=/tmp/main.sock main 'curl -s -o /dev/null -w "dokploy via tunnel: %{http_code}\n" http://127.0.0.1:13000/'
```

Expected: HTTP-код ответа Dokploy (200/302/404 — главное не `000`).

- [ ] **Step 4: Агенты в UI**

В `https://ci.ezheg.xyz` → Settings → Agents: `main-linux` (`host=main`), `mbp15-mac` (`host=mbp15`, platform `darwin/amd64`), `mbp15-docker` (`host=mbp15`, `linux/amd64`) — все online.

### Task 6: Проверка сборкой `kdr` (нужно «да» на push)

- [ ] **Step 1: Push**

```bash
cd ../kdr && git push origin master
```

Push идёт и в копию на `main` (симлинк `/var/lib/kdr-ci` → `/var/lib/ci/kdr`), и на GitHub.

- [ ] **Step 2: Сменить push-адрес копии на новый путь**

```bash
cd ../kdr
git remote set-url --push --delete origin ssh://main/var/lib/kdr-ci/mirror.git
git remote set-url --push --add origin ssh://main/var/lib/ci/kdr/mirror.git
git remote -v
```

Expected: два push-адреса: `ssh://main/var/lib/ci/kdr/mirror.git` и GitHub. Порядок важен: копия на `main` первой. Если GitHub оказался первым — удалить его и добавить снова (`--delete` + `--add`).

- [ ] **Step 3: Дождаться сборки**

В `ci.ezheg.xyz` → `kdr`: пайплайн коммита из задачи 3 — `check` на `main-linux`, `check-mac` на `mbp15-mac`. Оба зелёные. Если упали по путям — сравнить с задачей 3, не откатывать стек.

### Task 7: Один colima на `mbp15` (простой hlab ~1 минута — нужно «да»)

- [ ] **Step 1: Убрать лишние launchd-задания**

```bash
ssh mbp15 '
set -e
launchctl bootout gui/$(id -u)/com.ezheg.colima 2>/dev/null || launchctl unload ~/Library/LaunchAgents/com.ezheg.colima.plist 2>/dev/null || true
mv ~/Library/LaunchAgents/com.ezheg.colima.plist ~/backup-ci-2026-10-15/LaunchAgent-com.ezheg.colima.plist.moved
launchctl remove com.devtest.colima 2>/dev/null || true
launchctl list | grep -i colima || echo "no user colima jobs"'
```

Expected: `no user colima jobs` (системное задание `com.ezheg.colima` видно только через `sudo launchctl list`).

- [ ] **Step 2: Переключить LaunchDaemon на MacPorts colima**

```bash
ssh mbp15 '
set -e
sudo launchctl bootout system/com.ezheg.colima 2>/dev/null || true
sudo install -o root -g wheel -m 644 /tmp/com.ezheg.colima.plist /Library/LaunchDaemons/com.ezheg.colima.plist
sudo launchctl bootstrap system /Library/LaunchDaemons/com.ezheg.colima.plist
sleep 5; tail -3 ~/Library/Logs/colima.out.log'
```

Expected: `default уже запущена` (ВМ не перезапускалась).

- [ ] **Step 3: Проверить перезапуск (простой)**

```bash
ssh mbp15 'export PATH=/opt/local/bin:$PATH; colima stop -p default && sudo launchctl kickstart -k system/com.ezheg.colima; sleep 90; colima list; DOCKER_HOST=unix:///Users/devtest/.colima/default/docker.sock docker ps --format "{{.Names}} {{.Status}}"'
```

Expected: `default Running`, контейнеры Dokploy, fancyforia, dozzle, beszel-agent — Up. `curl -s -o /dev/null -w "%{http_code}" http://localhost:3000/` на mbp15 — не `000`. Агент `mbp15-docker` снова online.

- [ ] **Step 4: Убрать zerobrew colima**

```bash
ssh mbp15 'grep -rl /opt/zerobrew/bin/colima /Library/LaunchDaemons ~/Library/LaunchAgents ~/ci 2>/dev/null || echo "no references"; ls /opt/zerobrew/bin | grep -E "^(zb|zerobrew)$"'
```

Если ссылок нет: удалить пакет командой менеджера zerobrew, найденной выше (`zb uninstall colima` или `zerobrew uninstall colima`; проверить `--help`). Остальные пакеты zerobrew не трогать. Затем `ls /opt/zerobrew/bin/colima` → `No such file`.

### Task 8: Уборка (нужно «да»)

Только после зелёных сборок задачи 6 и проверки задачи 7.

- [ ] **Step 1: `main`**

```bash
ssh -o ControlPath=/tmp/main.sock main '
set -e
sudo rm /var/lib/kdr-ci
docker volume rm kdr-ci_woodpecker-data kdr-ci_woodpecker-agent
mv ~/kdr-ci ~/backup-ci-2026-10-15/kdr-ci-old
ls ~ ; docker volume ls | grep -E "ci_|kdr" '
```

Expected: остались только `ci_woodpecker-data`, `ci_woodpecker-agent`. `linux.Dockerfile` из старого каталога есть в репозитории `kdr` (`ci/linux.Dockerfile`).

- [ ] **Step 2: `mbp15`**

```bash
ssh mbp15 'mv ~/kdr-ci ~/backup-ci-2026-10-15/kdr-ci-old; ls ~/ci'
```

Резервные каталоги `~/backup-ci-2026-10-15` удалить через неделю стабильной работы.

### Task 9: Документация `hlab-infra` и push

**Files:**
- Modify: `../hlab-infra/CLAUDE.md` (раздел Topology: CI на `main`, ссылка на `ci/README.md`)
- Modify: `../hlab-infra/README.md` (таблица хостов: `main` — CI и будущие публичные проекты; дерево: `ci/`)

- [ ] **Step 1: Дописать в `../hlab-infra/CLAUDE.md` после абзаца Topology**

```markdown
CI: shared Woodpecker `ci.ezheg.xyz` on `main` (`~/ci`, compose from `ci/`, not Dokploy) plus agents
`mbp15-mac`/`mbp15-docker`; details and new-project steps in `ci/README.md`. `mbp15` runs exactly one
colima (MacPorts) with profile `default` for Dokploy, hlab services and the CI docker agent.
```

`CLAUDE.md` уже содержит чужие незакоммиченные правки: дописать только этот абзац и коммитить с `git add -p CLAUDE.md`, выбрав только свой hunk.

- [ ] **Step 2: README**

В таблицу «Раскладка» добавить строку:

```markdown
| `main` (VPS, Ubuntu 24.04, 4 CPU / 16 GB) | Общий CI (`ci/`), будущие публичные проекты |
```

В дерево структуры добавить строку `├── ci/             общий Woodpecker: compose для main, агенты/туннель/colima mbp15`.

- [ ] **Step 3: Commit и push (нужно «да»)**

```bash
cd ../hlab-infra
git add -p CLAUDE.md
git add README.md
git commit -m "docs: shared ci on main, single colima on mbp15"
git push origin master
```

Push в `hlab-infra` запускает autodeploy настроенных стеков Dokploy; `ci/` в Dokploy не подключён, поэтому деплой его не затронет.

---

## Критерии готовности части 1

1. `ci.ezheg.xyz` работает, история `kdr` на месте, три агента online с метками `host`.
2. Сборка `kdr` зелёная на `main-linux` и `mbp15-mac`; кэши в `/var/lib/ci/kdr`, `~/ci/cache`.
3. На `mbp15` один colima (MacPorts), один профиль `default`, одно launchd-задание; сервисы hlab работают после перезапуска.
4. На `main` `curl http://127.0.0.1:13000/` отвечает Dokploy через туннель.
5. Каталогов и volumes `kdr-ci` не осталось (кроме резервных копий).
