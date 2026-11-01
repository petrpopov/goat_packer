-- Append-only migration: «Проверить нарезкой» живёт внутри исходной упаковки. Проверка — внутренняя
-- дочерняя строка очереди (воркер и пакер те же), родитель ссылается на последнюю через check_job_id.
-- Дети не показываются в списке задач и удаляются вместе с родителем.
alter table queue_jobs add column parent_job_id varchar(36) references queue_jobs(id) on delete cascade;
alter table queue_jobs add column check_job_id varchar(36);
create index queue_jobs_parent_job_id_idx on queue_jobs(parent_job_id);
