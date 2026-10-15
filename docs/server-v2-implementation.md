# Серверный режим v2 — состояние на 2026-10-15

Реализация спецификации [public-service-architecture](superpowers/specs/2026-09-26-public-service-architecture.md)
без RabbitMQ. Запуск и обновление: [deploy/README.md](../deploy/README.md).

## Реализовано и покрыто тестами

- PostgreSQL + Flyway (V1, append-only V2) + JPA: владельцы, сессии, reset tokens, загрузки/blobs,
  задачи и attempts, артефакты, worker registry, лимиты, audit, очередь удаления объектов.
- Профиль `server` с `/api/v2`; незащищённые local endpoints в нём отключены.
- Гости, регистрация с переносом собственной гостевой истории, вход/выход, cookie-сессии, CSRF,
  серверные ADMIN-проверки; админка (аккаунты/гости, блокировка, отзыв сессий, одноразовый reset,
  очередь, workers, storage, параллелизм, пауза новых запусков).
- Очередь на PostgreSQL: global/per-owner/per-worker лимиты, идемпотентность, snapshot профиля,
  fences/leases, отмена, deadlines. После рестарта worker — подтверждённая остановка старых
  контейнеров, затем один повтор с новым fence; без подтверждения слот остаётся LOST.
- CPU-time admission (резерв timeout, возврат неиспользованного), суточные бюджеты гостя/аккаунта/глобальный.
- Owner-bound одноразовый SHA256 proof-of-work до приёма байтов (Web Worker, работает по HTTP/IP);
  rate limits; ограничения ZIP/XML; лимит control JSON.
- Изолированное исполнение: отдельный контейнер на задачу (read-only, network none, cap-drop ALL,
  CPU/RAM/PID limits, ограниченный tmpfs), экспорт только обычных файлов доверенным helper.
- S3 (SeaweedFS 4.47): private bucket, signed download, SHA256-dedup, attempt-scoped выводы,
  TTL гостей, reference-aware GC с fencing публикации.
- Закреплённые Linux-сборки Bambu Studio 2.8.2.61 и OrcaSlicer 2.4.2 (SHA256), headless Xvfb.

## Проверки

- Rust 146, backend 85 (включая настоящие PostgreSQL/SeaweedFS через Testcontainers и nginx/Tomcat
  origin-тест), frontend 10 + typecheck/lint/build.
- `deploy/smoke.py` (гость → PoW → upload → pack и schedule в настоящем runner → signed S3 download)
  проходит на Compose-стенде на macOS/ARM с эмуляцией amd64.

## Квалификация на Ubuntu 24.04 x86-64 (2026-10-15)

VPS 4 vCPU / 16 GiB, сервису выделено 3 CPU / 10 GiB → `configure.py` дал 1 слот (2 CPU, 4 GiB).
Стек привязан к loopback. `smoke.py` проходит. Реальные проекты через `/api/v2` с настройками по умолчанию
(resolution 0.25, attempts 4800):

| Проект | macOS (локальный слайсер) | Linux-сервер | Время на сервере | Пик RAM задачи |
|---|---|---|---|---|
| P1S, 7 цветов, башня | 16 | 16 | 151 с | 2.5 GiB |
| то же + `nozzle-clean global` | 16 | 16 | 152 с | 2.5 GiB |
| U1 (Orca), 4 цвета, башня | 14 (Orca 2.5.0-dev) | 15 (Orca 2.4.2) | 118 с | 2.5 GiB |
| A2L, поддержки | 116 | 108 | 46 с | 1.0 GiB |

- Все серверные 3MF нарезаются Bambu Studio / OrcaSlicer на macOS без конфликтов путей;
  `nozzle-clean` попадает в `layer_change_gcode`.
- A2L: та же версия Bambu Studio 2.8.2.61 на Linux даёт более широкое пятно первого слоя
  (поддержки/brim выступают на 3.0 мм против 2.2 мм на macOS), поэтому раскладка консервативнее.
  U1: разные версии Orca по-разному ставят башню. Результат проверен серверным слайсером;
  пользователь со слайсером другой версии должен нарезать файл сам и проверить.
- Проект, на котором башня снова меняется после разрешённой перепаковки, падает одинаково
  на macOS и Linux (штатная ошибка `evaluate_final_tower`, не дефект развёртывания).
- Образы и сборка занимают около 14 GB диска.

## Не завершено

1. Нагрузочное/abuse испытание на изолированном стенде.
2. Отдельная Spring worker JVM вместо лёгкого Rust node agent.
3. Вне объёма текущей выкладки: TLS/домен, бэкапы, SMTP-reset, метрики/дашборды, RabbitMQ.
   По HTTP пароли и cookies передаются без шифрования.
