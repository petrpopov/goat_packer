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

## Не завершено

1. Квалификация на целевом Ubuntu x86-64: U1/A2L/nozzle-clean/prime tower на Linux-слайсерах,
   замеры времени/RSS/scratch на реальном железе.
2. Нагрузочное/abuse испытание на изолированном стенде.
3. Отдельная Spring worker JVM вместо лёгкого Rust node agent.
4. Вне объёма текущей выкладки: TLS/домен, бэкапы, SMTP-reset, метрики/дашборды, RabbitMQ.
   По HTTP пароли и cookies передаются без шифрования.
