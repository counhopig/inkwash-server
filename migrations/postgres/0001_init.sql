-- 0001_init: full PostgreSQL schema. Keep in sync with `POSTGRES_TABLES`
-- in `src/db/schema.rs` (mirror of the same DDL with BIGINT/BIGSERIAL types).
CREATE TABLE IF NOT EXISTS accounts (
    id BIGSERIAL PRIMARY KEY,
    username TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    created_at BIGINT NOT NULL
);
CREATE TABLE IF NOT EXISTS sessions (
    token TEXT PRIMARY KEY,
    account_id BIGINT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    created_at BIGINT NOT NULL
);
CREATE TABLE IF NOT EXISTS devices (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    token TEXT NOT NULL UNIQUE,
    version BIGINT NOT NULL DEFAULT 0,
    created_at BIGINT NOT NULL,
    account_id BIGINT REFERENCES accounts(id) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS alarms (
    device_id TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
    local_id BIGINT NOT NULL,
    hour BIGINT NOT NULL,
    minute BIGINT NOT NULL,
    repeat_kind TEXT NOT NULL,
    once_year BIGINT,
    once_month BIGINT,
    once_day BIGINT,
    repeat_days TEXT,
    enabled BIGINT NOT NULL,
    label TEXT NOT NULL,
    PRIMARY KEY (device_id, local_id)
);
CREATE TABLE IF NOT EXISTS todos (
    device_id TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
    local_id BIGINT NOT NULL,
    text TEXT NOT NULL,
    done BIGINT NOT NULL,
    importance TEXT NOT NULL DEFAULT 'medium',
    due_year BIGINT,
    due_month BIGINT,
    due_day BIGINT,
    repeat_kind TEXT,
    repeat_days TEXT,
    PRIMARY KEY (device_id, local_id)
);
CREATE TABLE IF NOT EXISTS channels (
    id TEXT PRIMARY KEY,
    device_id TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    name TEXT NOT NULL,
    enabled BIGINT NOT NULL DEFAULT 1,
    token_hash TEXT,
    token_prefix TEXT,
    config_encrypted TEXT,
    config_version BIGINT NOT NULL DEFAULT 1,
    last_sync_at BIGINT,
    last_sync_error TEXT,
    sync_state TEXT,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_channels_device ON channels(device_id);
CREATE INDEX IF NOT EXISTS idx_channels_kind_enabled ON channels(kind, enabled);
CREATE TABLE IF NOT EXISTS inbox (
    id TEXT PRIMARY KEY,
    device_id TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
    channel_id TEXT NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    event_id TEXT,
    seq BIGINT NOT NULL,
    kind TEXT NOT NULL,
    priority TEXT NOT NULL DEFAULT 'normal',
    title TEXT NOT NULL,
    body TEXT,
    when_epoch BIGINT,
    source_ref TEXT,
    read BIGINT NOT NULL DEFAULT 0,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL,
    UNIQUE(device_id, seq),
    UNIQUE(channel_id, source_ref)
);
CREATE INDEX IF NOT EXISTS idx_inbox_device_seq ON inbox(device_id, seq DESC);
CREATE INDEX IF NOT EXISTS idx_inbox_device_read ON inbox(device_id, read, seq DESC);
CREATE TABLE IF NOT EXISTS device_sequences (
    device_id TEXT PRIMARY KEY REFERENCES devices(id) ON DELETE CASCADE,
    next_inbox_seq BIGINT NOT NULL
);