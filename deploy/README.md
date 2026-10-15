# Сборка и развёртывание

Образы собирает Woodpecker (`ci.ezheg.xyz`) прямо в Docker на `main`; реестра нет.

- Push в любую ветку: `.woodpecker/check.yaml` — тесты Rust, backend (Testcontainers), frontend, затем
  образы `goat-packer-*:branch-<ветка>` (перезаписываются следующим push, никуда не выкладываются).
- Тег `v*`: `.woodpecker/release.yaml` — `deploy/build-images.sh <тег>` собирает
  `goat-packer-{api,worker,gateway,runner-bambu,runner-orca}:<тег>`, черновик GitHub Release получает
  `goat-packer-<тег>-linux-x86_64.tar.gz` (CLI), хранятся образы трёх последних версий.

Выпуск версии: `git tag v0.2.0 && git push origin v0.2.0`. Черновик релиза публикуется руками:
GitHub → Releases → черновик → Edit → Publish release (на работу сервера публикация не влияет).

Базы слайсеров `goat-packer-slicer-{bambu,orca}:<версия>` описаны в `deploy/slicers.env` (URL, SHA256,
offset SquashFS) и собираются один раз; AppImage при сборке не запускается. Смена версии слайсера —
правка `slicers.env`, следующий релиз соберёт новую базу.

Руками на x86-64 машине с Docker: `sh deploy/build-images.sh dev`, затем
`GOAT_VERSION=dev docker compose --env-file deploy/.env -f deploy/compose.yml up -d`
(в `deploy/.env` — `DATABASE_PASSWORD`, `S3_SECRET_KEY`, `ADMIN_PASSWORD` и параметры ниже).
Развёртывание на сервер — через Dokploy (раздел добавляется в части 3).

Обработка задач идёт локально в контейнерах **без сети**; слайсеры на хосте не нужны.
При HTTP пароли и cookies передаются без шифрования.

## Ресурсы и исполнение

По умолчанию одна native задача:2CPU,4GiB RAM, hard tmpfs scratch2GiB,256 процессов,
30мин максимум,50MiB совокупных выходных файлов (включая диагностику). Export helper ограничен
128MiB/0.5CPU; запас control plane учтён в расчёте. Пакер ограничивает Rayon двумя потоками.
Автораcчёт использует CPU/RAM **Docker-хоста**, не предполагает фиксированную машину4GiB.
На shared-host явно укажите выделенный бюджет. Нулевое безопасное число слотов допустимо:
система не начинает задачи вместо рискованного принудительного N=1.

Worker — доверенный контроллер с Docker socket. Он не выполняет загруженный код в своей JVM.
Каждая задача имеет отдельный непривилегированный контейнер: read-only root, network none,
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

- Очередь максимум20 задач; на гостя2 активных, аккаунт4; одновременно одна задача владельца.
- CPU-time admission: резерв полного timeout; по умолчанию гость3600сек/сутки,
  аккаунт10800сек/сутки, глобально72000сек/сутки. Неиспользованный резерв возвращается.
- Гость:150MiB и24ч хранения после окончания; аккаунт1GiB без TTL полезных результатов.
  JSON/log аккаунтов удаляются через7дней. Общая storage quota ограничивает admission.
- До загрузки одноразовый owner-bound SHA256 proof-of-work в браузерном Web Worker.
  Работает и по HTTP/IP, без сторонних CAPTCHA/CDN. Есть общие/IP/owner rate limits,
  максимум2 одновременных upload intents,50MiB вход, временной лимит загрузки и64KiB control JSON.
- ZIP traversal/дубликаты путей/DTD/external entities/post-process commands отклоняются;
  лимит распакованного содержимого1GiB, XML-entry256MiB,10000 entries.
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
pack→schedule, скачивает3MF из S3, проверяет скрытие диагностики и удаляет свои тестовые задачи.
Не удаляет существующие данные. Для внешнего запуска укажите IP вместо127.0.0.1.

Backend logs — JSON/ECS с jobId/attemptFence; события slicer stdout/stderr передаются в job log
без неограниченного буфера. Docker logs ротируются10MiB×3. В обычном UI видны3MF/таблицы,
при `?debug=true` — ещё JSON/log. Админка показывает пользователей/гостей, блокировку,
одноразовый сброс пароля, фактические/разрешённые слоты и состояние worker.

```sh
TESTCONTAINERS_DOCKER_SOCKET_OVERRIDE=/var/run/docker.sock ./webapp/backend/gradlew -p webapp/backend test
./webapp/backend/gradlew -p webapp/backend nativeTest  # нужны собранные runner images
cd webapp/frontend && bun test tests && bun run typecheck && bun run lint && bun run build
```

Backend tests используют отдельные disposable PG/S3, не рабочую БД. `nativeTest` не скрывается
как успешный при отсутствии образов: их нужно собрать. Fixtures не ссылаются на Downloads.
Старый профиль `local` остаётся только для доверенного локального использования.

Gateway сохраняет внешний порт в X-Forwarded-Port, перезаписывает клиентские forwarded headers
и не отключает CORS/CSRF. Ошибка403 на localhost8088 проверяется настоящим nginx/Tomcat тестом.
За прокси, снимающим TLS (Pangolin), gateway получает `GOAT_PUBLIC_SCHEME=https` — схема задаётся
конфигурацией, клиентский `X-Forwarded-Proto` на неё не влияет. Подписанные ссылки на файлы идут по пути
`/goat/<ключ>` того же имени (`S3_PUBLIC_ENDPOINT` = публичный адрес приложения): gateway отдаёт его
хранилищу только на чтение, без изменения Host и пути, поэтому подпись S3 сходится. Отдельное имя
или порт для файлов не нужны.

### Параметры

Основные env: `WORKER_CPUS`, `WORKER_MEMORY_MIB`, `EXECUTION_CPU_BUDGET`,
`EXECUTION_MEMORY_MIB`, `MAX_PARALLEL` (начальный), `STORAGE_LIMIT_BYTES`, `JOB_TIMEOUT_SECONDS`,
`MAX_QUEUE`, `GLOBAL_COMPUTE_SECONDS`, `GOAT_VERSION` (версия образов, runner — той же версии).
Снижение лимита параллелизма в админке останавливает новые запуски, но не убивает уже работающие
задачи. ETA появляется после пяти сопоставимых успешных измерений и является диапазоном,
а не гарантированным сроком. Не повышайте memory/parallelism без выделенного ресурса.
