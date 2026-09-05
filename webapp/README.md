# Goat Packer Web

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
cd webapp/frontend && bun run typecheck && bun run build
```
