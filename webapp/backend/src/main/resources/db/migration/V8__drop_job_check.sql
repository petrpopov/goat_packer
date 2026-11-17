-- Append-only migration: «Проверить нарезкой» убрана из webapp (2026-11-17): серверная нарезка не доказывает
-- нарезку в GUI пользователя. Внутренние строки проверок (parent_job_id не null) удаляются так же, как их удаляло
-- удаление задачи: файлы их артефактов уходят в object_gc (если их не держит блоб), зарезервированный
-- вычислительный бюджет ещё идущих проверок возвращается (у завершённых он уже 0), строки удаляются
-- (артефакты, попытки и доступы — каскадом). Блоб состояния родителя, который проверка брала на вход, держится
-- артефактом родителя и уходит вместе с ним. Затем колонки проверки больше не нужны.
insert into object_gc(object_key,size_bytes,not_before)
 select a.object_key,a.size_bytes,now() from artifacts a join queue_jobs c on c.id=a.job_id
 where c.parent_job_id is not null and not exists(select 1 from blobs b where b.object_key=a.object_key)
 on conflict do nothing;

update compute_budgets b set reserved_seconds=greatest(0,b.reserved_seconds-r.seconds)
 from (select owner_id,budget_day,sum(compute_reserved_seconds) seconds from queue_jobs
       where parent_job_id is not null and budget_day is not null and compute_reserved_seconds>0
       group by owner_id,budget_day) r
 where b.owner_id=r.owner_id and b.day=r.budget_day;

update compute_budgets b set reserved_seconds=greatest(0,b.reserved_seconds-r.seconds)
 from (select budget_day,sum(compute_reserved_seconds) seconds from queue_jobs
       where parent_job_id is not null and budget_day is not null and compute_reserved_seconds>0
       group by budget_day) r
 where b.owner_id='GLOBAL' and b.day=r.budget_day;

delete from queue_jobs where parent_job_id is not null;

drop index if exists queue_jobs_parent_job_id_idx;
alter table queue_jobs drop column parent_job_id;
alter table queue_jobs drop column check_job_id;
