create table principals (
 id varchar(36) primary key, kind varchar(16) not null, login varchar(120) unique,
 password_hash varchar(255), role varchar(16) not null default 'USER', blocked boolean not null default false,
 block_reason varchar(500), reset_required boolean not null default false,
 created_at timestamptz not null, last_seen_at timestamptz not null,
 storage_quota bigint not null, version bigint not null default 0
);
create table auth_sessions (
 token_hash varchar(64) primary key, owner_id varchar(36) not null references principals(id),
 expires_at timestamptz not null, revoked boolean not null default false
);
create index sessions_owner on auth_sessions(owner_id);
create table password_resets (
 token_hash varchar(64) primary key, owner_id varchar(36) not null references principals(id),
 expires_at timestamptz not null, used_at timestamptz
);
create table platform_config (
 id integer primary key check(id=1), max_parallel integer not null check(max_parallel>=0),
 paused boolean not null default false, version bigint not null default 0
);
create table blobs (
 id varchar(36) primary key, sha256 varchar(64) not null, size_bytes bigint not null,
 object_key varchar(500) not null unique, created_at timestamptz not null,
 unique(sha256,size_bytes)
);
create table uploads (
 id varchar(36) primary key, owner_id varchar(36) not null references principals(id),
 original_name varchar(200) not null, expected_bytes bigint not null,
 state varchar(20) not null, blob_id varchar(36) references blobs(id),
 printer_model varchar(200), profile_id varchar(100), created_at timestamptz not null,
 expires_at timestamptz, version bigint not null default 0
);
create index uploads_owner on uploads(owner_id);
create table queue_jobs (
 id varchar(36) primary key, owner_id varchar(36) not null references principals(id),
 kind varchar(16) not null, status varchar(24) not null, input_blob_id varchar(36) not null references blobs(id),
 original_name varchar(200) not null, profile_id varchar(100) not null, profile_json text not null,
 params_json text not null, request_key varchar(100) not null, request_hash varchar(64) not null,
 read_token_hash varchar(64), phase varchar(32) not null, progress integer not null default 0,
 message varchar(1000), error varchar(2000), result_json text,
 created_at timestamptz not null, started_at timestamptz, finished_at timestamptz, expires_at timestamptz,
 worker_id varchar(100), fence bigint not null default 0, lease_until timestamptz, deadline_at timestamptz,
 cancel_requested boolean not null default false, reserved_bytes bigint not null default 0,
 version bigint not null default 0, unique(owner_id,request_key)
);
create index jobs_claim on queue_jobs(status,created_at,id);
create index jobs_owner on queue_jobs(owner_id,created_at desc);
create index jobs_expiry on queue_jobs(expires_at);
create table artifacts (
 id varchar(36) primary key, job_id varchar(36) not null references queue_jobs(id) on delete cascade,
 name varchar(200) not null, download_name varchar(240) not null, object_key varchar(500) not null unique,
 size_bytes bigint not null, debug boolean not null, created_at timestamptz not null,
 unique(job_id,name)
);
create table worker_nodes (
 id varchar(100) primary key, boot_id varchar(36) not null, cpu_limit double precision not null,
 memory_limit_bytes bigint not null, capacity integer not null, active_count integer not null,
 last_seen_at timestamptz not null, mode varchar(20) not null
);
create table audit_events (
 id varchar(36) primary key, actor_id varchar(36), target_id varchar(36), action varchar(100) not null,
 details varchar(1000) not null, created_at timestamptz not null
);
create table request_limits (
 key varchar(180) primary key, window_start timestamptz not null, used integer not null
);
create table read_grants (
 owner_id varchar(36) not null references principals(id), job_id varchar(36) not null references queue_jobs(id) on delete cascade,
 expires_at timestamptz not null, primary key(owner_id,job_id)
);
create table object_gc (
 object_key varchar(500) primary key, size_bytes bigint not null, not_before timestamptz not null
);
