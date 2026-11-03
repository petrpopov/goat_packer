-- Append-only migration: persistent probe cache of the packer (colored_3mf_packer --cache-dir). The runner is
-- untrusted and has no shared directory, so the trusted worker keeps the entries as objects
-- `probe-cache/<owner>/<group>/<name>`: per owner (a guest is an owner too), so one account's job can never feed
-- another's cache. `group_key` = SHA-256 of the job input (the model); `name` = the packer's entry file name.
create table probe_cache_entries (
 owner_id varchar(36) not null, group_key varchar(64) not null, name varchar(100) not null,
 object_key varchar(500) not null unique, size_bytes bigint not null,
 created_at timestamptz not null, last_used_at timestamptz not null,
 primary key (owner_id, group_key, name)
);
create index probe_cache_owner_used on probe_cache_entries(owner_id, last_used_at);
create index probe_cache_used on probe_cache_entries(last_used_at);
