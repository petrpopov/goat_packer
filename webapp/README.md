# Goat Packer Web

## Новый серверный режим и админка

Реализован отдельный режим `server`: PostgreSQL/Flyway/JPA, защищённые гостевые/пользовательские
сессии, очередь без RabbitMQ, S3 и админка. Инструкция и ограничения —
[deploy/README.md](../deploy/README.md). Ниже описан сохранённый локальный режим;
в нём нет аккаунтов/админки и его нельзя публиковать в Интернет.


Веб-обёртка над `colored_3mf_packer` и `daily_scheduler`: загрузка 3MF с одним объектом →
максимально упакованный 3MF (фаза 1), затем расписание на день → мультипластинный 3MF +
таблица работ в CSV/XLSX (фаза 2). Без БД: джобы в памяти, файлы во временном каталоге
(`${java.io.tmpdir}/goat-packer-web/`, автоочистка по TTL).

## Запуск

Сначала собери бинарники (если ещё не):

```bash
cargo build --release          # из корня репозитория
```

Бэкенд (Kotlin + Spring Boot, порт 19190):

```bash
cd webapp/backend
./gradlew bootRun              # пути к бинарникам/профилям — дефолты ../../target/release и ../../profiles
```

Фронтенд (Vite + React, порт 3007, прокси /api → 19190):

```bash
cd webapp/frontend
bun install
bun run dev
```

Открыть http://localhost:3007.

## Переменные окружения бэкенда

| Переменная | Что переопределяет | Дефолт |
|---|---|---|
| `GOAT_PACKER_BIN` | путь к colored_3mf_packer | `../../target/release/colored_3mf_packer` |
| `GOAT_SCHEDULER_BIN` | путь к daily_scheduler | `../../target/release/daily_scheduler` |
| `GOAT_PROFILES_DIR` | каталог профилей принтеров | `../../profiles` |
| `GOAT_JOBS_ROOT` | каталог джобов | `${java.io.tmpdir}/goat-packer-web/jobs` |
| `PORT` | порт бэкенда | 19190 |

## API

- `GET /api/profiles` — профили принтеров из `profiles/`.
- `POST /api/pack/jobs` — multipart: `file` (.3mf, один объект) + `profileId`, `clearance`,
  `resolution`, `attempts`, `candidates`, `seed`, `nozzleClean` → `{"jobId"}`.
- `POST /api/schedule/jobs` — multipart: `file` или `packJobId` (артефакт фазы 1) +
  `profileId`, `dayStart`, `nightStart`, `horizonEnd` (HH:MM, часы могут быть >24),
  `turnaroundMin`, `prepMin`, `benchMode` (`adaptive`|`full`) → `{"jobId"}`.
- `GET /api/jobs/{id}` — статус/прогресс/результат (поллинг).
- `GET /api/jobs/{id}/artifacts/{name}` — скачивание артефакта.
- `DELETE /api/jobs/{id}` — отменить и удалить.

Ошибки — JSON `{"code","message","error"}` с русским описанием.

## Тесты

```bash
cd webapp/backend && ./gradlew test    # с фейковыми бинарниками, реальный слайсер не нужен
cd webapp/frontend && bun test tests && bun run typecheck && bun run lint && bun run build
```

## Проверка и исправления webapp (2026-09-26)

Подробности: [аудит webapp](../docs/webapp-audit.md).

- Загрузка реальных 3MF из BambuStudio использует ZIP central directory, включая PNG
  STORED + data descriptors. Ошибка чтения архива больше не маскируется под отсутствие принтера.
- Статус задачи и файл сохраняются при переключении вкладок. Кнопка отмены завершает
  процессы и предотвращает запуск уже удалённых задач из очереди. После перезапуска сервера
  задачи всё ещё не восстанавливаются (хранилище в памяти); UI явно сообщает об этом по 404.
- Планировщик копирует входной артефакт, поэтому удаление исходной pack-задачи не ломает очередь.
- Seed и candidates сохранены только для совместимости API; UI больше не предлагает случайную
  «лотерею» раскладок. Поиск детерминированный, ползунок управляет конечным набором стратегий.
- Локальные CORS origins по умолчанию: `http://localhost:3007`, `http://127.0.0.1:3007`.
  Для отдельного стенда задайте Spring property `app.allowed-origins` (список точных origins),
  а у Vite `GOAT_API_TARGET=http://localhost:PORT`. Не включайте wildcard для локального API.

После изменения Kotlin требуется перезапустить backend (`./gradlew bootRun`);
при запуске через JAR сначала `./gradlew bootJar`, затем запуск нового JAR.
Фронтенд в dev обновляется через HMR; при статической раздаче нужна новая `bun run build`.
