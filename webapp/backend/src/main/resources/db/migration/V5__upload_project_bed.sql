-- Append-only migration: стол из 3MF неизвестного принтера (JSON ProjectBed) — фронт рисует его
-- и предлагает как «свой стол»; null — принтер распознан или в проекте нет прямоугольного стола.
alter table uploads add column project_bed_json text;
