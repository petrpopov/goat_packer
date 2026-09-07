# Спека: веб-приложение поверх colored_3mf_packer и daily_scheduler

Дата: 2026-09-06. Статус: реализуется.

## Зачем

Упаковка и планирование сегодня — ручные вызовы CLI-бинарников из терминала. Веб-приложение
даёт тот же результат без консоли: загрузил 3MF с одним объектом — получил максимально
упакованный 3MF; задал расписание — получил мультипластинный 3MF + таблицу работ (CSV/XLSX).

## Границы

- Без БД, без авторизации. Состояние — jobs в памяти, файлы — во временном каталоге
  (`${java.io.tmpdir}/goat-packer-web/`), автоочистка устаревших джобов.
- Оборачиваются готовые release-бинарники Rust (`colored_3mf_packer`, `daily_scheduler`);
  никакая логика упаковки/планирования не дублируется в Kotlin.
- Профили принтеров — только существующие `profiles/*.json` (Bambu A2L, Bambu P1S, Snapmaker U1).
  Новые принтеры добавляются профилем, не кодом.
- Один пользователь за раз; очередь джобов последовательная (упаковка ест все ядра через rayon).

## Фазы UI

1. **Упаковка**: загрузка .3mf (один объект, любая покраска) → выбор профиля + параметры
   (clearance, resolution, attempts, candidates, seed, nozzle-clean) → прогресс-бар по фазам →
   результат: сколько объектов упаковано, ссылки на скачивание (упакованный 3MF,
   verification.json, лог). Кнопка «Отправить в расписание» передаёт артефакт в фазу 2.
2. **Расписание**: источник — файл с фазы 1 или своя загрузка (плотный однопластинный 3MF) →
   окно дня (start/night/horizon HH:MM), turnaround, prep-минуты, режим бенчмарка
   (adaptive/full) → прогресс → таблица работ (время старта/финиша, размер партии, филамент,
   накопительно, overnight) + итоги; скачивание: schedule.csv, schedule.xlsx, plates.3mf, report.json.

## Контракт API (v1)

- `GET /api/profiles` — список профилей из `profiles/`: id, name, slicerKind, bedSize.
- `POST /api/pack/jobs` — multipart `file` + поля `profileId, clearance, resolution, attempts,
  candidates, seed, nozzleClean` → `{jobId}`. Валидация: .3mf, ≤200 МБ, числовые поля.
  Дефолты: attempts 4800 / candidates 2400 («Сильный» уровень ползунка усилия в UI:
  Быстро 500/400, Стандарт 2000/1000, Сильно 4800/2400, Максимум 12000/2400); seed без
  значения — случайный на прогон (результат поиска сильно зависит от seed, повторный запуск
  может упаковать плотнее; закреплённый seed даёт воспроизводимость).
- `GET /api/jobs/{id}` — статус: `RUNNING/SUCCEEDED/FAILED` + `phase, progressPercent,
  phaseMessage, error, artifacts[{name, sizeBytes, downloadUrl}], pack: {objectCount,
  bedBounds, profileId}` / `schedule: {jobs[], totalObjects, totalFilamentGrams,
  completionClock}`.
- `POST /api/schedule/jobs` — multipart `file` **или** `packJobId` (взять выход фазы 1) + поля
  `profileId, dayStart, nightStart, horizonEnd, turnaroundMin, prepMin, benchMode` → `{jobId}`.
- `GET /api/jobs/{id}/artifacts/{name}` — скачивание артефакта (Content-Disposition attachment).
- `DELETE /api/jobs/{id}` — удалить джоб и его файлы.
- Ошибки: JSON `{"code","message","error"}` на русском (стиль ozonware), 400/404/409/500.

Прогресс парсится из stdout/stderr бинарников:
- packer: `probing…` (5%), `pack_footprint: N/M attempts…` (10–70% по N/M, репак сбрасывает),
  `slicing final plate…` (80%), `wrote` (100%).
- scheduler: `slicing sample i/j: batch N/M` и `refining: slicing batch N` (бенчмаркинг 10–80%),
  `writing … plate(s)` (90%), summary (100%).

## Стек и стиль

- Бэкенд: Kotlin 2.0 + Spring Boot 3.4 (web + validation), Gradle Kotlin DSL, Jackson Kotlin,
  без JPA/без Lombok, классы-сервисы без интерфейсов, `@RestControllerAdvice` с плоскими
  кодами ошибок, логи на русском, порт 19190. Стиль — как ozonware/backend.
- Фронтенд: Vite + React 18 + TypeScript strict, bun; React Query (поллинг статуса джоба),
  vanilla CSS с OKLCH-токенами (тёмная тема первична), IBM Plex Sans/JetBrains Mono, классы
  `btn/btn-primary/input/pill/table-compact/mono`; порт 3007, прокси `/api` → 19190.
  Тексты на русском. Стиль — как ozonware/frontend.
- XLSX собирается на бэкенде Apache POI (тот же набор колонок, что CSV).

## Не делаем

- Мультипользовательность, очереди с приоритетами, докачка, редактор расписания мышью,
- хранение истории между перезапусками, авторизация.
