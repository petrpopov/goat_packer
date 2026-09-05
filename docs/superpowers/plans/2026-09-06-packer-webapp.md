# План: веб-приложение поверх packer + scheduler

Дата: 2026-09-06. Спека: `docs/superpowers/specs/2026-09-06-packer-webapp-design.md`.

## Волна 0 — Rust CLI (готово, этот коммит)

1. `scheduler.rs`: сетка листов `ceil(sqrt(n))` вместо `round` (баг «объекты уехали за лист»);
   `BatchCurve` + `least_squares_line` + `next_refinement_count` для адаптивного бенчмаркинга.
2. `daily_scheduler.rs`: `--bench-mode adaptive|full` (дефолт adaptive), `--day-start/--night-start/
   --horizon-end HH:MM`, `origin: measured|interpolated` в отчёте, подписи листов `N pcs @ HH:MM`.
3. Тесты: `tests/scheduler_plate_grid.rs` (синтетический 3MF, 2 и 5 листов), юнит-тесты кривой и
   выборки. Живая проверка: goat1 A2L 30 объектов → schedule + plates.3mf → слайс каждой пластины
   реальным BambuStudio CLI.

## Волна 1 — бэкенд `webapp/backend/` (Kotlin/Spring Boot)

Файлы (порядок сборки):

1. `build.gradle.kts`, `settings.gradle.kts` — kotlin("jvm")/spring плагины как в ozonware,
   зависимости: web, validation, jackson-module-kotlin, poi-ooxml; порт 19190.
2. `PackerWebApplication.kt` + `application.yml` (пути к бинарникам и профилям через env с
   дефолтами на репо: `app.packer-bin`, `app.scheduler-bin`, `app.profiles-dir`,
   `app.jobs-root` = `${java.io.tmpdir}/goat-packer-web`, `app.job-ttl-hours: 12`).
3. `config/AsyncConfig.kt` — executor `jobExecutor` (1 поток; очередь 16; CallerRuns).
4. `exception/GlobalExceptionHandler.kt` + доменные исключения (стиль ozonware, русские тексты).
5. `job/JobStore.kt` — ConcurrentHashMap джобов; `job/JobRunner.kt` — запуск процесса
   (ProcessBuilder, merge stdout/stderr, строчный парсер прогресса, лог-файл в каталоге джоба);
   `job/JobCleanup.kt` — @Scheduled очистка по TTL.
6. `profile/PrinterProfiles.kt` — чтение profiles/*.json (Jackson, маппинг в DTO).
7. `pack/PackService.kt` — сборка команды packer + парсинг verification.json; `api/PackController.kt`.
8. `schedule/ScheduleService.kt` — сборка команды scheduler + чтение CSV/report.json;
   `schedule/XlsxWriter.kt` (POI); `api/ScheduleController.kt`.
9. `api/JobController.kt` — статус + артефакты + DELETE.
10. Тесты: контекст поднимается, валидация полей отдаёт 400, profiles отдаёт 3 профиля,
    прогресс-парсер переводит строки бинарника в проценты (Unit-тесты без реального слайсера).

## Волна 2 — фронтенд `webapp/frontend/` (React/Vite/TS)

1. `package.json` + `vite.config.ts` + `tsconfig.json` + токены/стили (`src/styles/`):
   tokens.css (OKLCH, тёмная тема первична), core.css (btn, input, pill, table, progress),
   app.css. Шрифты @fontsource IBM Plex Sans / JetBrains Mono.
2. `api/http.ts` — fetch-обёртка (_ApiError_, blob-скачивание), `api/jobs.ts` — типы + функции.
3. `components/`: ProgressBar, Field (label+input), FileDrop (drag&drop), StatusPill,
   ArtifactList, ToastHost (+ zustand store).
4. `pages/PackPage.tsx` — форма + прогресс + результат + «в расписание».
5. `pages/SchedulePage.tsx` — форма (или источник из фазы 1) + прогресс + таблица работ +
   скачивания.
6. `App.tsx` — две вкладки, общий header; `main.tsx`.
7. `bun run typecheck` и `bun run build` — 0 ошибок.

## Волна 3 — сквозная проверка

1. `cargo build --release` уже собран; бэкенд `./gradlew test`; фронт `bun run typecheck && build`.
2. Поднять бэкенд + фронт-dev; прогнать полный цикл curl'ом: pack (маленькие attempts) →
   job → artifacts; schedule (adaptive, окна по умолчанию) → таблица + csv/xlsx/plates.
3. Визуальная проверка UI (скриншоты двух фаз).
4. README `webapp/` (как запускать).
