-- 0001_init: full SQLite schema. Keep in sync with `SQLITE_TABLES` in
-- `src/db/schema.rs`, which `migrate_legacy_integer_ids` reuses when it
-- recreates tables after the pre-UUID-era rebuild.
CREATE TABLE IF NOT EXISTS accounts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    username TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS sessions (
    token TEXT PRIMARY KEY,
    account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS devices (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    token TEXT NOT NULL UNIQUE,
    version INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    account_id INTEGER REFERENCES accounts(id) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS alarms (
    device_id TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
    local_id INTEGER NOT NULL,
    hour INTEGER NOT NULL,
    minute INTEGER NOT NULL,
    repeat_kind TEXT NOT NULL,
    once_year INTEGER,
    once_month INTEGER,
    once_day INTEGER,
    repeat_days TEXT,
    enabled INTEGER NOT NULL,
    label TEXT NOT NULL,
    PRIMARY KEY (device_id, local_id)
);
CREATE TABLE IF NOT EXISTS todos (
    device_id TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
    local_id INTEGER NOT NULL,
    text TEXT NOT NULL,
    done INTEGER NOT NULL,
    importance TEXT NOT NULL DEFAULT 'medium',
    due_year INTEGER,
    due_month INTEGER,
    due_day INTEGER,
    repeat_kind TEXT,
    repeat_days TEXT,
    PRIMARY KEY (device_id, local_id)
);
CREATE TABLE IF NOT EXISTS channels (
    id TEXT PRIMARY KEY,
    device_id TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    name TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    token_hash TEXT,
    token_prefix TEXT,
    config_encrypted TEXT,
    config_version INTEGER NOT NULL DEFAULT 1,
    last_sync_at INTEGER,
    last_sync_error TEXT,
    sync_state TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_channels_device ON channels(device_id);
CREATE INDEX IF NOT EXISTS idx_channels_kind_enabled ON channels(kind, enabled);
CREATE TABLE IF NOT EXISTS inbox (
    id TEXT PRIMARY KEY,
    device_id TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
    channel_id TEXT NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    event_id TEXT,
    seq INTEGER NOT NULL,
    kind TEXT NOT NULL,
    priority TEXT NOT NULL DEFAULT 'normal',
    title TEXT NOT NULL,
    body TEXT,
    when_epoch INTEGER,
    source_ref TEXT,
    read INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE(device_id, seq),
    UNIQUE(channel_id, source_ref)
);
CREATE INDEX IF NOT EXISTS idx_inbox_device_seq ON inbox(device_id, seq DESC);
CREATE INDEX IF NOT EXISTS idx_inbox_device_read ON inbox(device_id, read, seq DESC);
CREATE TABLE IF NOT EXISTS device_sequences (
    device_id TEXT PRIMARY KEY REFERENCES devices(id) ON DELETE CASCADE,
    next_inbox_seq INTEGER NOT NULL
);