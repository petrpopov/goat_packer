# Часть 3. Деплой goat_packer через Dokploy на main — план реализации

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** тег `v*` приводит к работающей версии на `main`: Woodpecker собирает образы и через Dokploy (на `mbp15`) запускает их на `main`; владелец открывает `https://goat.ezheg.xyz` через Pangolin SSO. `main` виден в Beszel, старый ручной стенд убран.

**Architecture:** Спецификация `docs/superpowers/specs/2026-10-15-ci-dokploy-deploy-design.md`, часть 3. Dokploy ходит на `main` по SSH под root (требование Dokploy) — только по своему ключу и только с домашнего адреса `37.113.208.189`; setup-скрипт урезан: Swarm, `dokploy-network`, каталог приложений, без сборщиков и без traefik. Compose-стек `goat-packer` берёт `deploy/compose.yml` из публичного репозитория, образы — локальные `goat-packer-*:${GOAT_VERSION}`. Наружу — только Pangolin (newt на `main`, siteId 2), порты стека на `127.0.0.1`. TLS снимается на `cloud`, поэтому gateway получает явный `GOAT_PUBLIC_SCHEME=https`.

**Tech Stack:** Dokploy v0.30.5 API (`x-api-key`, на `mbp15:3000`, с `main` — `127.0.0.1:13000` через туннель CI), nginx 1.28 templates (envsubst), Pangolin 1.18 (sqlite на `cloud`), Beszel agent, Woodpecker 3.19.

---

## Факты

- Dokploy API: ключ — строка `dokploy api key …: <tok>` в `../hlab-infra/env`; вызовы `ssh mbp15 "curl -s -H 'x-api-key: $tok' http://localhost:3000/api/<proc>"`; GET с параметрами в query, POST с JSON. Проект `hlab` = `TfRln1UIhlkNE0a3ct4t7`, окружение `production` = `hRjRI4B7TnJ2Wb9nGgTlq`. Серверов и SSH-ключей пока нет.
- `main`: sshd порт 2783, `PermitRootLogin no`, `AllowUsers petrpopov`, ufw `LIMIT` на 2783. Домашний адрес (и `mbp15`, и рабочий Mac) — `37.113.208.189`.
- Pangolin (`ssh cloud`, `sudo sqlite3 /opt/pangolin/config/db/db.sqlite`): сайт `main` = siteId 2 (online); образец строк — ресурс 11 `kdr-ci-main` (`ci.ezheg.xyz`). Свободные internalPort: 40010, 40011. Перед записью — резервная копия БД. Новый ресурс требует строк в `resources`, `targets`, `roleResources` (roleId 1). Новому target нужен перезапуск newt на `main`: `docker restart ci-newt-1` (newt в стеке `ci`).
- Новый стек: порты `127.0.0.1:18088` (web) и `127.0.0.1:18089` (файлы), `WORKER_ID=main-1`, scratch `/srv/goat-packer/scratch` — не пересекаются со старым стендом (`~/goat_packer`, порты 8088/8091, `node-1`, `/srv/goat/scratch`), пока тот не удалён.
- Образы `goat-packer-*:v0.1.0` уже в Docker на `main`.
- Gateway: `X-Forwarded-Proto` сейчас `$scheme` (http за Pangolin) — браузерный `Origin: https://goat.ezheg.xyz` был бы отклонён. Тест `GatewayOriginIntegrationTest` требует, чтобы клиентские forwarded-заголовки игнорировались, поэтому схема задаётся конфигурацией, а не заголовком.
- SSH на `main` — только через мультиплексированное соединение (`-o ControlMaster=auto -o ControlPath=/tmp/main.sock -o ControlPersist=4h`).

## Нужно от владельца

- Beszel: KEY и TOKEN системы `main` (Add system в `metrics.ezheg.xyz`).
- Секрет Woodpecker `dokploy_token` (значение — Dokploy API key), events `tag`.
- Браузерная проверка `goat.ezheg.xyz` после SSO.

---

### Task 1: Публичная схема в gateway (TDD)

**Files:**
- Modify: `webapp/backend/src/test/kotlin/com/goatpacker/web/server/GatewayOriginIntegrationTest.kt`
- Modify: `deploy/nginx.conf`
- Modify: `deploy/Dockerfile.frontend`

- [ ] **Step 1: Тест на https за прокси**

В `GatewayOriginIntegrationTest` вынести запуск nginx в функцию с параметром схемы и добавить второй тест. Конфиг теперь шаблон: тест подставляет `${GOAT_PUBLIC_SCHEME}` сам, как это делает entrypoint nginx.

```kotlin
    private fun withGateway(publicScheme: String, check: (origin: String, mappedOrigin: String, client: HttpClient, csrf: String) -> Unit) {
        Testcontainers.exposeHostPorts(port)
        val config = Files.readString(Path.of("../../deploy/nginx.conf"))
            .replace("\${GOAT_PUBLIC_SCHEME}", publicScheme)
            .replace("http://api:19190", "http://host.testcontainers.internal:$port")
            .replace("http://storage:8333", "http://host.testcontainers.internal:$port")
        val configPath = directory.resolve("nginx.conf")
        Files.writeString(configPath, config)
        class Nginx : GenericContainer<Nginx>("nginx:1.28-alpine")
        Nginx().withExposedPorts(80)
            .withCopyFileToContainer(MountableFile.forHostPath(configPath), "/etc/nginx/conf.d/default.conf")
            .use { gateway ->
                gateway.start()
                val mapped = "${gateway.host}:${gateway.getMappedPort(80)}"
                val cookies = CookieManager(null, CookiePolicy.ACCEPT_ALL)
                val client = HttpClient.newBuilder().cookieHandler(cookies).build()
                val csrfResponse = client.send(
                    HttpRequest.newBuilder(URI("http://$mapped/api/v2/auth/csrf")).GET().build(),
                    HttpResponse.BodyHandlers.ofString(),
                )
                assertEquals(200, csrfResponse.statusCode())
                val csrf = json.readTree(csrfResponse.body())["token"].asText()
                check("${publicScheme.ifEmpty { "http" }}://$mapped", "http://$mapped", client, csrf)
            }
    }

    private fun guest(client: HttpClient, target: String, csrf: String, from: String, includeCsrf: Boolean = true, spoofForwarded: Boolean = false): HttpResponse<String> {
        val request = HttpRequest.newBuilder(URI("$target/api/v2/auth/guest"))
            .header("Origin", from).header("Content-Type", "application/json")
        if (includeCsrf) request.header("X-XSRF-TOKEN", csrf)
        if (spoofForwarded) request
            .header("Forwarded", "host=evil.invalid;proto=https")
            .header("X-Forwarded-Host", "evil.invalid")
            .header("X-Forwarded-Port", "443")
            .header("X-Forwarded-Proto", "https")
        return client.send(request.POST(HttpRequest.BodyPublishers.ofString("{}")).build(), HttpResponse.BodyHandlers.ofString())
    }

    @Test
    fun `mapped gateway port is same origin but hostile origin and missing csrf remain forbidden`() =
        withGateway("") { origin, target, client, csrf ->
            val accepted = guest(client, target, csrf, origin, spoofForwarded = true)
            assertEquals(200, accepted.statusCode(), accepted.body())
            assertEquals("GUEST", json.readTree(accepted.body())["kind"].asText())
            val current = client.send(HttpRequest.newBuilder(URI("$target/api/v2/auth/me")).GET().build(), HttpResponse.BodyHandlers.ofString())
            assertEquals(200, current.statusCode(), current.body())
            assertEquals(json.readTree(accepted.body())["id"], json.readTree(current.body())["id"])
            assertEquals(403, guest(client, target, csrf, "https://evil.invalid", spoofForwarded = true).statusCode())
            val noCsrf = guest(client, target, csrf, origin, includeCsrf = false)
            assertEquals(403, noCsrf.statusCode())
            assertEquals("FORBIDDEN", json.readTree(noCsrf.body())["code"].asText())
        }

    /** Pangolin terminates TLS and talks plain HTTP to the gateway: the public scheme is configuration. */
    @Test
    fun `configured https public scheme accepts the https origin and still rejects plain http`() =
        withGateway("https") { origin, target, client, csrf ->
            assertTrue(origin.startsWith("https://"))
            val accepted = guest(client, target, csrf, origin)
            assertEquals(200, accepted.statusCode(), accepted.body())
            assertEquals(403, guest(client, target, csrf, origin.replaceFirst("https://", "http://")).statusCode())
        }
```

Прежнее тело теста `mapped gateway port …` целиком заменяется этим кодом (логика проверок та же).

- [ ] **Step 2: Тест падает**

```bash
cd webapp/backend && TESTCONTAINERS_DOCKER_SOCKET_OVERRIDE=/var/run/docker.sock ./gradlew test --tests '*GatewayOriginIntegrationTest*'
```

Expected: FAIL `configured https public scheme …` (403 вместо 200: nginx передаёт `X-Forwarded-Proto: http`).

- [ ] **Step 3: nginx**

В `deploy/nginx.conf` заменить первый `map`:

```nginx
# Preserve the browser-facing port across Docker's WEB_PORT:80 mapping.
# $server_port is only the internal listener (80), not necessarily the public port.
map $http_host $goat_forwarded_port {
    default $server_port;
    ~:(?<goat_authority_port>[0-9]+)$ $goat_authority_port;
}
```

на

```nginx
# Public scheme when TLS is terminated upstream (Pangolin): GOAT_PUBLIC_SCHEME=https (substituted by the
# nginx image entrypoint from /etc/nginx/templates). Empty = this listener's scheme. Configuration, not a
# client header: a client-supplied X-Forwarded-Proto must never change what the backend trusts.
map "${GOAT_PUBLIC_SCHEME}" $goat_proto {
    "" $scheme;
    default "${GOAT_PUBLIC_SCHEME}";
}
map $http_host $goat_authority_port {
    default "";
    ~:(?<goat_port>[0-9]+)$ $goat_port;
}
# Preserve the browser-facing port across Docker's WEB_PORT:80 mapping; an https URL without a port is 443.
# $server_port is only the internal listener (80), not necessarily the public port.
map "$goat_authority_port$goat_proto" $goat_forwarded_port {
    ~^(?<goat_explicit>[0-9]+) $goat_explicit;
    https 443;
    default $server_port;
}
```

и в `location /api/` строку `proxy_set_header X-Forwarded-Proto $scheme;` на `proxy_set_header X-Forwarded-Proto $goat_proto;`.

- [ ] **Step 4: Шаблон в образе**

`deploy/Dockerfile.frontend`: заменить `COPY deploy/nginx.conf /etc/nginx/conf.d/default.conf` на

```dockerfile
# Шаблон: entrypoint nginx подставляет определённые переменные окружения (GOAT_PUBLIC_SCHEME) в conf.d.
COPY deploy/nginx.conf /etc/nginx/templates/default.conf.template
ENV GOAT_PUBLIC_SCHEME=""
```

- [ ] **Step 5: Тесты проходят** (команда шага 2). Expected: оба теста PASS.

- [ ] **Step 6: Commit**

```bash
git add deploy/nginx.conf deploy/Dockerfile.frontend webapp/backend/src/test/kotlin/com/goatpacker/web/server/GatewayOriginIntegrationTest.kt
git commit -m "Gateway: configured public scheme for TLS-terminating proxies (Pangolin)"
```

### Task 2: compose для Dokploy

**Files:** Modify: `deploy/compose.yml`

- [ ] **Step 1:** у `gateway` добавить

```yaml
    environment:
      # https за Pangolin/traefik, которые снимают TLS до gateway; пусто — прямой http.
      GOAT_PUBLIC_SCHEME: ${GOAT_PUBLIC_SCHEME:-}
```

- [ ] **Step 2: Проверка**

```bash
GOAT_VERSION=v0 DATABASE_PASSWORD=x S3_SECRET_KEY=x ADMIN_PASSWORD=x GOAT_PUBLIC_SCHEME=https docker compose -f deploy/compose.yml config | grep -A1 GOAT_PUBLIC_SCHEME
```

Expected: `GOAT_PUBLIC_SCHEME: https`.

- [ ] **Step 3: Commit** `git commit -am "Compose: pass GOAT_PUBLIC_SCHEME to the gateway"` (только `deploy/compose.yml`).

### Task 3: smoke за https-прокси

**Files:** Modify: `deploy/smoke.py`

- [ ] **Step 1:** добавить параметры `--public-origin` (Origin, который видит backend; по умолчанию = `--url`) и `--files-via` (локальный адрес файлового порта; подписанная ссылка вызывается по нему с исходным `Host`, подпись SigV4 покрывает host и путь, не схему). Secure-cookie по http cookiejar не отправляет — после каждого ответа снимать флаг `secure`:

```python
    def __init__(self, origin, public_origin=None, files_via=None):
        self.origin=origin.rstrip('/');self.public=(public_origin or origin).rstrip('/');self.files_via=files_via
        self.jar=http.cookiejar.CookieJar()
        self.http=urllib.request.build_opener(urllib.request.HTTPCookieProcessor(self.jar))
        self.call('GET','/auth/csrf');self.call('POST','/auth/guest')
```

в `call`: `h={'Origin':self.public}`; после успешного `self.http.open` — `for c in self.jar: c.secure=False` (локальный http-вызов за TLS-прокси; на живой домен smoke не ходит).

в `verify_downloads` вместо `urllib.request.urlopen(d['url'],timeout=60)`:

```python
            url=d['url'];headers={}
            if self.files_via:
                parts=urllib.parse.urlsplit(url);headers['Host']=parts.netloc
                url=self.files_via.rstrip('/')+parts.path+('?'+parts.query if parts.query else '')
            with urllib.request.urlopen(urllib.request.Request(url,headers=headers),timeout=60) as response:
```

`import urllib.parse`; в `main()` — `p.add_argument('--public-origin')`, `p.add_argument('--files-via')`, `Client(args.url,args.public_origin,args.files_via)`.

- [ ] **Step 2: Синтаксис** `python3 -m py_compile deploy/smoke.py && echo ok`.

- [ ] **Step 3: Commit** `git commit -m "smoke: run behind a TLS-terminating proxy (public origin, files via loopback)" deploy/smoke.py`.

### Task 4: Доступ Dokploy к main

- [ ] **Step 1: Ключ Dokploy**

```bash
tok=$(grep -iE '^dokploy api key' ../hlab-infra/env | sed -E 's/.*:[[:space:]]*//' | head -1)
ssh mbp15 "curl -s -X POST -H 'x-api-key: $tok' -H 'Content-Type: application/json' -d '{}' http://localhost:3000/api/sshKey.generate" > /tmp/dokploy-key.json
python3 -c "import json;d=json.load(open('/tmp/dokploy-key.json'));print(d.keys())"
```

Ответ содержит пару ключей; затем `sshKey.create` с `name: "main-root"`, `privateKey`, `publicKey`, `organizationId` (из `project.all`). Сохранить `sshKeyId`. Публичный ключ — в `/tmp/dokploy-main.pub`; приватный нигде не печатать, файл удалить после `sshKey.create`.

- [ ] **Step 2: sshd на main** — drop-in и AllowUsers (резервная копия, проверка `sshd -t` до перезагрузки, текущая сессия не рвётся):

```bash
scp -o ControlPath=/tmp/main.sock /tmp/dokploy-main.pub main:/tmp/
ssh -o ControlPath=/tmp/main.sock main '
set -e
sudo cp -a /etc/ssh/sshd_config /etc/ssh/sshd_config.bak-2026-10-16
f=$(sudo grep -rl "^AllowUsers" /etc/ssh/sshd_config /etc/ssh/sshd_config.d/ | head -1)
sudo sed -i "s/^AllowUsers petrpopov$/AllowUsers petrpopov root@37.113.208.189/" "$f"
printf "%s\n" "# Dokploy (mbp15) manages this host as root: key only, only from the home address." "Match Address 37.113.208.189" "    PermitRootLogin prohibit-password" | sudo tee /etc/ssh/sshd_config.d/90-dokploy.conf >/dev/null
sudo install -d -m 700 /root/.ssh
echo "from=\"37.113.208.189\",no-agent-forwarding,no-X11-forwarding $(cat /tmp/dokploy-main.pub)" | sudo tee -a /root/.ssh/authorized_keys >/dev/null
sudo chmod 600 /root/.ssh/authorized_keys
sudo sshd -t && sudo systemctl reload ssh
sudo ufw insert 1 allow from 37.113.208.189 to any port 2783 proto tcp
sudo ufw status numbered | head -6'
```

Expected: `sshd -t` без ошибок; первое правило ufw — `2783/tcp ALLOW IN 37.113.208.189`.

- [ ] **Step 3: Проверка с mbp15** — Dokploy проверит сам в Task 5 (`server.validate`); вход `petrpopov` с Mac по-прежнему работает: `ssh -o ControlPath=none main true && echo ok`.

### Task 5: Сервер main в Dokploy

- [ ] **Step 1:** `server.create`: `name: "main"`, `ipAddress: "195.208.61.28"`, `port: 2783`, `username: "root"`, `sshKeyId`, `serverType: "deploy"`, `description: "goat_packer и будущие публичные проекты"`. Сохранить `serverId`.
- [ ] **Step 2: Урезанный setup-скрипт** — `server.update` с `command` (свой скрипт вместо стандартного):

```bash
set -e
# Docker уже установлен. Без Nixpacks/Railpack/Buildpacks/RClone: стеки приходят готовыми образами.
docker info --format '{{.Swarm.LocalNodeState}}' | grep -q active || docker swarm init --advertise-addr 127.0.0.1
docker network inspect dokploy-network >/dev/null 2>&1 || docker network create --driver overlay --attachable dokploy-network
mkdir -p /etc/dokploy/applications /etc/dokploy/compose /etc/dokploy/logs
echo done
```

Если API не принимает `command` — открыть сервер в UI Dokploy → Setup Server → Modify Script и вставить тот же текст (владелец или через браузер).
- [ ] **Step 3:** `server.setup` (или кнопка Setup Server), затем на main: `docker info --format '{{.Swarm.LocalNodeState}}'` → `active`; `docker network ls | grep dokploy-network`; `ss -ltn | grep -E ':(80|443) '` → пусто; `docker ps -a | grep -i traefik` → пусто (если Dokploy всё же поднял traefik — `docker rm -f dokploy-traefik` и записать в README).
- [ ] **Step 4:** Woodpecker и стек `ci` продолжают работать: `docker ps --format '{{.Names}} {{.Status}}' | grep ci-`, `curl -s -o /dev/null -w '%{http_code}' https://ci.ezheg.xyz/` → 302.

### Task 6: Стек goat-packer в Dokploy и первый деплой

- [ ] **Step 1:** `compose.create`: `name: "goat-packer"`, `environmentId: "hRjRI4B7TnJ2Wb9nGgTlq"`, `serverId`, `composeType: "docker-compose"`. Затем `compose.update`: `sourceType: "git"`, `customGitUrl: "https://github.com/petrpopov/goat_packer.git"`, `customGitBranch: "master"`, `composePath: "./deploy/compose.yml"`, `autoDeploy: false`, `env` — строки ниже (пароли сгенерировать `openssl rand -base64 32 | tr -d '/+=' | cut -c1-32`, в вывод не печатать):

```dotenv
GOAT_VERSION=v0.1.0
DATABASE_PASSWORD=<сгенерировать>
S3_SECRET_KEY=<сгенерировать>
ADMIN_LOGIN=admin
ADMIN_PASSWORD=<сгенерировать>
COOKIE_SECURE=true
GOAT_PUBLIC_SCHEME=https
S3_PUBLIC_ENDPOINT=https://goat-files.ezheg.xyz
BIND_ADDRESS=127.0.0.1
WEB_PORT=18088
OBJECT_PORT=18089
WORKER_ID=main-1
SCRATCH_ROOT=/srv/goat-packer/scratch
MAX_PARALLEL=1
EXECUTION_CPU_BUDGET=2
EXECUTION_MEMORY_MIB=4096
WORKER_CPUS=2
WORKER_MEMORY_MIB=4096
STORAGE_LIMIT_BYTES=10737418240
```

- [ ] **Step 2:** на main `sudo mkdir -p /srv/goat-packer/scratch`.
- [ ] **Step 3:** `compose.deploy`, ждать `compose.one` → `composeStatus: done`; на main `docker ps --format '{{.Names}} {{.Status}}' | grep goat-packer` — пять сервисов, api/worker/postgres healthy; `curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:18088/` → 200.
- [ ] **Step 4: smoke** на main (python3 есть на хосте):

```bash
ssh -o ControlPath=/tmp/main.sock main 'cd /tmp && rm -rf gp-smoke && git clone -q --depth=1 https://github.com/petrpopov/goat_packer.git gp-smoke && python3 gp-smoke/deploy/smoke.py --url http://127.0.0.1:18088 --public-origin https://127.0.0.1:18088 --files-via http://127.0.0.1:18089; rm -rf gp-smoke'
```

Host запроса — `127.0.0.1:18088`, а схема задана `GOAT_PUBLIC_SCHEME=https`, поэтому backend ждёт Origin `https://127.0.0.1:18088`. Expected: `PASS: guest/CSRF/PoW/upload/queue/native/S3/debug`.

### Task 7: Pangolin

- [ ] **Step 1:** резервная копия `sudo cp /opt/pangolin/config/db/db.sqlite /opt/pangolin/config/db/db.sqlite.bak-2026-10-16` на `cloud`.
- [ ] **Step 2:** по образцу ресурса 11 (`SELECT * FROM resources WHERE resourceId=11`, `SELECT * FROM targets WHERE resourceId=11`, `SELECT * FROM roleResources WHERE resourceId=11`) вставить два ресурса со всеми колонками образца, меняя: `niceId` `goat-main` / `goat-files-main`, `name`, `subdomain` `goat` / `goat-files`, `fullDomain` `goat.ezheg.xyz` / `goat-files.ezheg.xyz`, `sso=1`; targets: `siteId=2`, `ip=127.0.0.1`, `port` 18088 / 18089, `internalPort` 40010 / 40011, `method=http`; `roleResources(roleId=1, resourceId=<новый>)`.
- [ ] **Step 3:** перезапуск newt на main: `docker restart ci-newt-1`; через минуту `curl -s -o /dev/null -w '%{http_code}' https://goat.ezheg.xyz/` → 302 (SSO), `https://goat-files.ezheg.xyz/` → 302.
- [ ] **Step 4: Владелец в браузере:** вход через SSO на `goat.ezheg.xyz`, гостевая упаковка реального 3MF, скачивание результата (ссылка на `goat-files.ezheg.xyz`). Если скачивание упирается в SSO второго имени — перейти на скачивание через тот же домен (отдельная правка gateway, не в этом плане).

### Task 8: Деплой из релиза

**Files:** Modify: `.woodpecker/release.yaml`

- [ ] **Step 1:** после `prune old images` добавить шаг (Dokploy API доступен на хосте `main` по `127.0.0.1:13000` через туннель CI, поэтому вызов идёт контейнером в сети хоста):

```yaml
  # Dokploy (mbp15) переключает стек на эту версию; API доступен на main как 127.0.0.1:13000 через туннель CI.
  - name: deploy
    image: docker:29-cli
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock
    environment:
      DOKPLOY_TOKEN:
        from_secret: dokploy_token
      COMPOSE_ID: <composeId из задачи 6>
    commands:
      - self=$(grep -o '/containers/[0-9a-f]\{64\}' /proc/self/mountinfo | head -1 | cut -d/ -f3) && test -n "$self"
      - >-
        docker run --rm --network host --volumes-from "$self" -w "$CI_WORKSPACE"
        -e DOKPLOY_TOKEN -e COMPOSE_ID -e CI_COMMIT_TAG alpine:3.22
        sh -c 'apk add -q curl jq && sh deploy/dokploy-deploy.sh'
      - >-
        docker run --rm --network host --volumes-from "$self" -w "$CI_WORKSPACE" python:3.13-alpine
        python deploy/smoke.py --url http://127.0.0.1:18088 --public-origin https://127.0.0.1:18088 --files-via http://127.0.0.1:18089
```

- [ ] **Step 2: `deploy/dokploy-deploy.sh`** (CI-шаг релиза, shell + curl + jq): читает `compose.one`, ставит `GOAT_VERSION=<тег>` в `env`, `compose.update`, `compose.deploy`, ждёт `composeStatus` `done` (ошибка при `error`, таймаут 10 мин).

```sh
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
```

В шаге `deploy` первая команда `docker run` — `alpine:3.22 sh -c 'apk add -q curl jq && sh deploy/dokploy-deploy.sh'` (с `-e DOKPLOY_TOKEN -e COMPOSE_ID -e CI_COMMIT_TAG`).

- [ ] **Step 3: Commit** `.woodpecker/release.yaml`, `deploy/dokploy-deploy.sh`.

### Task 9: Beszel на main

- [ ] **Step 1:** в Dokploy: `compose.create` `name: "beszel-agent-main"`, `serverId` main, источник GitHub App `-70zZaI4yeBjpdUaZA-f2` (репозиторий `hlab-infra`, ветка `master`, `composePath: ./beszel/docker-compose.yml`), env `KEY=…`, `TOKEN=…` от владельца; `compose.deploy`.
- [ ] **Step 2:** в `metrics.ezheg.xyz` система `main` online, есть CPU/RAM/диск.

### Task 10: Уборка, документация, первый полный релиз

- [ ] **Step 1: Старый стенд на main** — только после зелёного smoke задачи 6:

```bash
ssh -o ControlPath=/tmp/main.sock main 'cd ~/goat_packer && docker compose --env-file deploy/.env -f deploy/compose.yml down -v --remove-orphans; cd ~ && rm -rf ~/goat_packer; sudo rm -rf /srv/goat; docker image rm goat-packer-api:latest goat-packer-worker:latest goat-packer-gateway:latest goat-runner-bambu:2.8.2.61 goat-runner-orca:2.4.2; docker image prune -f >/dev/null; df -h / | tail -1'
```

(старый стенд содержит только тестовые данные квалификации).
- [ ] **Step 2: `deploy/README.md`** — раздел «Развёртывание через Dokploy»: сервер `main` (root по ключу только с домашнего адреса, урезанный setup, без traefik), стек `goat-packer` (источник, переменные, порты 18088/18089), Pangolin `goat.ezheg.xyz`/`goat-files.ezheg.xyz` с SSO, релиз → деплой, откат сменой `GOAT_VERSION` + Deploy, переход на свой домен (включить traefik на main, домены в Dokploy, 80/443 в ufw, `GOAT_PUBLIC_SCHEME`, `S3_PUBLIC_ENDPOINT`, снять SSO).
- [ ] **Step 3: `hlab-infra`** — README: таблица портов `main` (goat 18088/40010, goat-files 18089/40011); `CLAUDE.md` (только свой абзац, как в части 1): «`main` is a Dokploy deploy server (root via Dokploy key only from home IP, minimal setup, no traefik); public exposure of main stays off until a domain is bought».
- [ ] **Step 4: Release** — слить в `master` (fast-forward), тег `v0.1.1`; пайплайн: check → release → deploy → smoke зелёные; в Dokploy `GOAT_VERSION=v0.1.1`.

---

## Критерии готовности части 3

1. `https://goat.ezheg.xyz` после SSO работает: упаковка и скачивание.
2. Тег `v*` сам разворачивает версию на `main` и проверяет её smoke.
3. На `main` снаружи открыт только SSH (root — только по ключу Dokploy с домашнего адреса); 80/443 не слушаются.
4. `main` в Beszel.
5. Старого ручного стенда нет; свободно ≥ 10 GB на `/`.
