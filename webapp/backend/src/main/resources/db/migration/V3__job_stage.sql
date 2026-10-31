-- Append-only migration: текущий этап пакера (JSON JobStage) для степпера прогресса.
alter table queue_jobs add column stage_json text;
