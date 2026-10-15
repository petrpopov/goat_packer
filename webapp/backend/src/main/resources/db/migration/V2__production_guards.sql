-- Append-only migration: V1 has already been applied to user deployments.
alter table queue_jobs add column attempt_count integer not null default 0;
alter table queue_jobs add column available_at timestamptz not null default now();
alter table queue_jobs add column compute_reserved_seconds bigint not null default 0;
alter table queue_jobs add column runner_image varchar(255);
update queue_jobs set attempt_count=1 where fence>0;
create table job_attempts (
 id varchar(36) primary key, job_id varchar(36) not null references queue_jobs(id) on delete cascade,
 fence bigint not null, worker_id varchar(100) not null, boot_id varchar(36) not null,
 started_at timestamptz not null, finished_at timestamptz, status varchar(24) not null,
 message varchar(2000), unique(job_id,fence)
);
create table compute_budgets (
 owner_id varchar(36) not null, day date not null, used_seconds bigint not null default 0,
 reserved_seconds bigint not null default 0, primary key(owner_id,day)
);
alter table queue_jobs add column budget_day date;
create table bot_challenges (
 id varchar(36) primary key, owner_id varchar(36) not null references principals(id),
 salt varchar(100) not null, difficulty integer not null, expires_at timestamptz not null, used boolean not null default false
);
create index challenge_expiry on bot_challenges(expires_at);
alter table uploads add column writer_token varchar(36);
create table operational_state (id integer primary key check(id=1), maintenance boolean not null default false);
insert into operational_state(id,maintenance) values(1,false);
alter table worker_nodes add column host_id varchar(100) not null default '';
alter table object_gc add column deleting boolean not null default false;
