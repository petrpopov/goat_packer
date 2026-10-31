-- Append-only migration: последняя раскладка пакера (JSON layout-события) для сцены стола.
-- Отдельно от stage_json: живёт и после успеха, не обрезается.
alter table queue_jobs add column layout_json text;
