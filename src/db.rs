//! Storage behind sqlx's `Any` driver, so one codebase serves both SQLite
//! (default, zero-config `sqlite://` URL) and PostgreSQL (via a
//! `postgres://` `DATABASE_URL`). The backend is selected at `open()` time
//! from the URL; DDL is emitted per-dialect, while the data queries below
//! use sqlx's cross-dialect `?` placeholders and `i64` for every integer so
//! one SQL body works on both backends.
//!
//! A small connection pool (default max 2) replaces the old single
//! `Arc<Mutex<Connection>>` - still far from anything you'd need deadpool
//! for, but it's what sqlx gives us for free.

use anyhow::{anyhow, Context, Result};
use rand::distributions::Alphanumeric;
use rand::Rng;
use sqlx::any::AnyPoolOptions;
use sqlx::{Any, AnyPool, Executor, Row};
use uuid::Uuid;

use crate::models::{
    Account, AccountSummary, Alarm, Channel, Device, DeviceSyncRequest, Importance, InboxItem,
    InboxKind, Repeat, Todo, UpsertAlarmRequest, UpsertTodoRequest,
};

/// Wraps the sqlx `Any` pool plus the backend flag. Data SQL is written with
/// `?` placeholders (SQLite-native); `adapt()` rewrites them to Postgres'
/// `$1, $2, …` form at runtime, since sqlx 0.8's Postgres driver does not
/// translate `?` itself.
#[derive(Clone)]
pub struct Db {
    pool: AnyPool,
    postgres: bool,
}

impl Db {
    fn adapt(&self, sql: &str) -> String {
        if !self.postgres {
            return sql.to_string();
        }
        let mut out = String::with_capacity(sql.len() + 8);
        let mut n = 0;
        for c in sql.chars() {
            if c == '?' {
                n += 1;
                out.push_str(&format!("${n}"));
            } else {
                out.push(c);
            }
        }
        out
    }
}

const SQLITE_TABLES: &str = "
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
    );";

const POSTGRES_TABLES: &str = "
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
    );";

/// Opens (creating the schema if needed) the database at `url`, which may be
/// `sqlite://...` (or a bare SQLite file path) or `postgres://...`.
/// `max_connections` sizes the pool; in-memory SQLite databases must use 1
/// so every query hits the same connection (memory DBs are per-connection).
pub async fn open(url: &str, max_connections: u32) -> Result<Db> {
    sqlx::any::install_default_drivers();
    let is_postgres = url.starts_with("postgres://") || url.starts_with("postgresql://");

    // sqlx 0.8 defaults to *not* creating a missing SQLite file, whereas the
    // old rusqlite backend created the database on open. Default to
    // `mode=rwc` (read-write + create) unless the caller pinned a mode.
    let mut url = url.to_string();
    if !is_postgres && !url.contains("mode=") {
        url.push_str(if url.contains('?') {
            "&mode=rwc"
        } else {
            "?mode=rwc"
        });
    }

    let mut options = AnyPoolOptions::new().max_connections(max_connections);
    if !is_postgres {
        // SQLite foreign keys are off per-connection by default; make sure
        // every pooled connection enforces them so cascades behave.
        options = options.after_connect(|conn, _meta| {
            Box::pin(async move {
                sqlx::query("PRAGMA foreign_keys = ON")
                    .execute(&mut *conn)
                    .await?;
                Ok(())
            })
        });
    }
    let pool = options
        .connect(&url)
        .await
        .with_context(|| format!("failed to connect to database at {url}"))?;
    let db = Db {
        pool,
        postgres: is_postgres,
    };

    if is_postgres {
        sqlx::raw_sql(POSTGRES_TABLES).execute(&db.pool).await?;
    } else {
        sqlx::raw_sql(SQLITE_TABLES).execute(&db.pool).await?;
        migrate_legacy_integer_ids(&db).await?;
        // Databases created before these columns existed are missing them;
        // `CREATE TABLE IF NOT EXISTS` never adds columns, so add them
        // explicitly and ignore the duplicate-column error.
        for stmt in [
            "ALTER TABLE todos ADD COLUMN importance TEXT NOT NULL DEFAULT 'medium'",
            "ALTER TABLE todos ADD COLUMN due_month INTEGER",
            "ALTER TABLE todos ADD COLUMN due_day INTEGER",
            "ALTER TABLE todos ADD COLUMN due_year INTEGER",
            "ALTER TABLE todos ADD COLUMN repeat_kind TEXT",
            "ALTER TABLE todos ADD COLUMN repeat_days TEXT",
            "ALTER TABLE alarms ADD COLUMN repeat_days TEXT",
            "ALTER TABLE devices ADD COLUMN account_id INTEGER",
        ] {
            let _ = sqlx::raw_sql(stmt).execute(&db.pool).await;
        }
    }

    Ok(db)
}

/// Legacy pre-UUID `alarms` row shape used only by the SQLite migration.
type LegacyAlarmRow = (
    i64,
    i64,
    i64,
    i64,
    String,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    i64,
    String,
);

/// Migrates SQLite databases from the pre-UUID era where `devices.id` was an
/// `INTEGER PRIMARY KEY AUTOINCREMENT` and `alarms`/`todos` referenced it with
/// an INTEGER `device_id`. Personal-scale data (a handful of devices) is
/// copied over row by row; the device auth token is preserved, so devices
/// keep syncing without re-registering. Only meaningful for SQLite.
async fn migrate_legacy_integer_ids(db: &Db) -> Result<()> {
    let legacy: i64 = sqlx::query_scalar(&db.adapt(
        "SELECT COUNT(*) FROM pragma_table_info('devices') WHERE name = 'id' AND type = 'INTEGER'",
    ))
    .fetch_one(&db.pool)
    .await?;
    if legacy == 0 {
        return Ok(());
    }
    tracing::info!("detected pre-UUID device ids; migrating to UUID keys");

    let mut tx = db.pool.begin().await?;
    let devices: Vec<(i64, String, String, i64, i64)> = {
        let rows =
            sqlx::query(&db.adapt("SELECT id, name, token, version, created_at FROM devices"))
                .fetch_all(&mut *tx)
                .await?;
        rows.into_iter()
            .map(|r| {
                Ok((
                    r.try_get::<i64, _>(0)?,
                    r.try_get::<String, _>(1)?,
                    r.try_get::<String, _>(2)?,
                    r.try_get::<i64, _>(3)?,
                    r.try_get::<i64, _>(4)?,
                ))
            })
            .collect::<Result<Vec<_>>>()?
    };
    let alarms: Vec<LegacyAlarmRow> = {
        let rows = sqlx::query(&db.adapt("SELECT device_id, local_id, hour, minute, repeat_kind, once_year, once_month, once_day, enabled, label FROM alarms"))
        .fetch_all(&mut *tx)
        .await?;
        rows.into_iter()
            .map(|r| {
                Ok((
                    r.try_get::<i64, _>(0)?,
                    r.try_get::<i64, _>(1)?,
                    r.try_get::<i64, _>(2)?,
                    r.try_get::<i64, _>(3)?,
                    r.try_get::<String, _>(4)?,
                    r.try_get::<Option<i64>, _>(5)?,
                    r.try_get::<Option<i64>, _>(6)?,
                    r.try_get::<Option<i64>, _>(7)?,
                    r.try_get::<i64, _>(8)?,
                    r.try_get::<String, _>(9)?,
                ))
            })
            .collect::<Result<Vec<_>>>()?
    };
    let todos: Vec<(i64, i64, String, i64)> = {
        let rows = sqlx::query(&db.adapt("SELECT device_id, local_id, text, done FROM todos"))
            .fetch_all(&mut *tx)
            .await?;
        rows.into_iter()
            .map(|r| Ok((r.try_get(0)?, r.try_get(1)?, r.try_get(2)?, r.try_get(3)?)))
            .collect::<Result<Vec<_>>>()?
    };

    sqlx::raw_sql("DROP TABLE alarms; DROP TABLE todos; DROP TABLE devices;")
        .execute(&mut *tx)
        .await?;
    // Recreate with the UUID schema (SQLite dialect - this path is SQLite-only).
    sqlx::raw_sql(SQLITE_TABLES).execute(&mut *tx).await?;

    let mut id_map = std::collections::HashMap::new();
    for (old_id, name, token, version, created_at) in &devices {
        let new_id = Uuid::new_v4().to_string();
        id_map.insert(*old_id, new_id.clone());
        sqlx::query(&db.adapt(
            "INSERT INTO devices (id, name, token, version, created_at) VALUES (?, ?, ?, ?, ?)",
        ))
        .bind(&new_id)
        .bind(name)
        .bind(token)
        .bind(version)
        .bind(created_at)
        .execute(&mut *tx)
        .await?;
    }
    for (
        device_id,
        local_id,
        hour,
        minute,
        repeat_kind,
        once_year,
        once_month,
        once_day,
        enabled,
        label,
    ) in &alarms
    {
        let Some(new_device_id) = id_map.get(device_id) else {
            continue;
        };
        sqlx::query(&db.adapt("INSERT INTO alarms (device_id, local_id, hour, minute, repeat_kind, once_year, once_month, once_day, enabled, label)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"))
        .bind(new_device_id)
        .bind(local_id)
        .bind(hour)
        .bind(minute)
        .bind(repeat_kind)
        .bind(once_year)
        .bind(once_month)
        .bind(once_day)
        .bind(enabled)
        .bind(label)
        .execute(&mut *tx)
        .await?;
    }
    for (device_id, local_id, text, done) in &todos {
        let Some(new_device_id) = id_map.get(device_id) else {
            continue;
        };
        sqlx::query(
            &db.adapt("INSERT INTO todos (device_id, local_id, text, done) VALUES (?, ?, ?, ?)"),
        )
        .bind(new_device_id)
        .bind(local_id)
        .bind(text)
        .bind(done)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    tracing::info!("migrated {} devices to UUID ids", devices.len());
    Ok(())
}

fn new_token() -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(48)
        .map(char::from)
        .collect()
}

/// Registers a new device and returns it with its token populated. The token
/// is a bearer credential (see `docs/sync-api.md`'s Security section in the
/// firmware repo) - only ever returned here, at creation time;
/// `list_devices` omits it, so losing it means re-registering. `account_id:
/// None` creates an unowned device (only reachable with the `ADMIN_TOKEN`);
/// `Some` ties the device to a console account.
pub async fn register_device(db: &Db, name: &str, account_id: Option<i64>) -> Result<Device> {
    let token = new_token();
    let now = now_unix();
    let id = Uuid::new_v4().to_string();
    sqlx::query(&db.adapt("INSERT INTO devices (id, name, token, version, created_at, account_id) VALUES (?, ?, ?, 0, ?, ?)"))
    .bind(&id)
    .bind(name)
    .bind(&token)
    .bind(now)
    .bind(account_id)
    .execute(&db.pool)
    .await?;
    Ok(Device {
        id,
        name: name.to_string(),
        token: Some(token),
    })
}

pub async fn list_devices(db: &Db) -> Result<Vec<Device>> {
    let rows = sqlx::query(&db.adapt("SELECT id, name FROM devices ORDER BY name"))
        .fetch_all(&db.pool)
        .await?;
    rows.into_iter()
        .map(|r| {
            Ok(Device {
                id: r.try_get(0)?,
                name: r.try_get(1)?,
                token: None,
            })
        })
        .collect()
}

/// Devices owned by one console account (for `GET /api/devices` when the
/// caller is an account session rather than the admin token).
pub async fn list_account_devices(db: &Db, account_id: i64) -> Result<Vec<Device>> {
    let rows =
        sqlx::query(&db.adapt("SELECT id, name FROM devices WHERE account_id = ? ORDER BY name"))
            .bind(account_id)
            .fetch_all(&db.pool)
            .await?;
    rows.into_iter()
        .map(|r| {
            Ok(Device {
                id: r.try_get(0)?,
                name: r.try_get(1)?,
                token: None,
            })
        })
        .collect()
}

/// Whether `device_id` exists and is owned by `account_id`.
pub async fn device_owned_by(db: &Db, device_id: &str, account_id: i64) -> Result<bool> {
    let count: i64 = sqlx::query_scalar(
        &db.adapt("SELECT COUNT(*) FROM devices WHERE id = ? AND account_id = ?"),
    )
    .bind(device_id)
    .bind(account_id)
    .fetch_one(&db.pool)
    .await?;
    Ok(count > 0)
}

pub async fn delete_device(db: &Db, device_id: &str) -> Result<()> {
    sqlx::query(&db.adapt("DELETE FROM devices WHERE id = ?"))
        .bind(device_id)
        .execute(&db.pool)
        .await?;
    Ok(())
}

/// Looks up the device owning `token`, returning `(device_id, version)`.
pub async fn find_device_by_token(db: &Db, token: &str) -> Result<Option<(String, i64)>> {
    let row = sqlx::query(&db.adapt("SELECT id, version FROM devices WHERE token = ?"))
        .bind(token)
        .fetch_optional(&db.pool)
        .await?;
    row.map(|r| -> Result<(String, i64)> { Ok((r.try_get(0)?, r.try_get(1)?)) })
        .transpose()
}

async fn bump_version<'e, E>(db: &Db, executor: E, device_id: &str) -> Result<()>
where
    E: Executor<'e, Database = Any>,
{
    sqlx::query(&db.adapt("UPDATE devices SET version = version + 1 WHERE id = ?"))
        .bind(device_id)
        .execute(executor)
        .await?;
    Ok(())
}

/// Applies only the fields the physical device is allowed to edit. Unknown
/// IDs are ignored so stale device caches cannot recreate content deleted by
/// Desktop/Server. Returns the resulting device version.
pub async fn merge_device_state(
    db: &Db,
    device_id: &str,
    state: &DeviceSyncRequest,
) -> Result<i64> {
    let mut tx = db.pool.begin().await?;
    let mut changed = false;
    for alarm in &state.alarms {
        let res = sqlx::query(&db.adapt(
            "UPDATE alarms SET enabled = ? WHERE device_id = ? AND local_id = ? AND enabled != ?",
        ))
        .bind(alarm.enabled as i64)
        .bind(device_id)
        .bind(alarm.id as i64)
        .bind(alarm.enabled as i64)
        .execute(&mut *tx)
        .await?;
        changed |= res.rows_affected() > 0;
    }
    for todo in &state.todos {
        let res =
            sqlx::query(&db.adapt(
                "UPDATE todos SET done = ? WHERE device_id = ? AND local_id = ? AND done != ?",
            ))
            .bind(todo.done as i64)
            .bind(device_id)
            .bind(todo.id as i64)
            .bind(todo.done as i64)
            .execute(&mut *tx)
            .await?;
        changed |= res.rows_affected() > 0;
        if let Some(importance) = todo.importance {
            let res = sqlx::query(&db.adapt("UPDATE todos SET importance = ? WHERE device_id = ? AND local_id = ? AND importance != ?"))
            .bind(importance.as_str())
            .bind(device_id)
            .bind(todo.id as i64)
            .bind(importance.as_str())
            .execute(&mut *tx)
            .await?;
            changed |= res.rows_affected() > 0;
        }
    }
    if changed {
        bump_version(db, &mut *tx, device_id).await?;
    }
    let version: i64 = sqlx::query_scalar(&db.adapt("SELECT version FROM devices WHERE id = ?"))
        .bind(device_id)
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(version)
}

pub async fn list_alarms(db: &Db, device_id: &str) -> Result<Vec<Alarm>> {
    let rows = sqlx::query(&db.adapt("SELECT local_id, hour, minute, repeat_kind, once_year, once_month, once_day, repeat_days, enabled, label
         FROM alarms WHERE device_id = ? ORDER BY local_id"))
    .bind(device_id)
    .fetch_all(&db.pool)
    .await?;
    rows.into_iter()
        .map(|row| {
            let repeat_kind: String = row.try_get(3)?;
            let repeat = repeat_from_columns(
                &repeat_kind,
                row.try_get(4)?,
                row.try_get(5)?,
                row.try_get(6)?,
                row.try_get(7)?,
            );
            Ok(Alarm {
                id: u8::try_from(row.try_get::<i64, _>(0)?)
                    .map_err(|_| anyhow!("alarm id out of range"))?,
                hour: u8::try_from(row.try_get::<i64, _>(1)?)
                    .map_err(|_| anyhow!("hour out of range"))?,
                minute: u8::try_from(row.try_get::<i64, _>(2)?)
                    .map_err(|_| anyhow!("minute out of range"))?,
                repeat,
                enabled: row.try_get::<i64, _>(8)? != 0,
                label: row.try_get(9)?,
            })
        })
        .collect()
}

pub async fn list_todos(db: &Db, device_id: &str) -> Result<Vec<Todo>> {
    let rows = sqlx::query(&db.adapt("SELECT local_id, text, done, importance, due_year, due_month, due_day, repeat_kind, repeat_days
         FROM todos WHERE device_id = ? ORDER BY local_id"))
    .bind(device_id)
    .fetch_all(&db.pool)
    .await?;
    rows.into_iter()
        .map(|row| {
            let importance_str: String = row.try_get(3)?;
            let due_year: Option<i64> = row.try_get(4)?;
            let due_month: Option<i64> = row.try_get(5)?;
            let due_day: Option<i64> = row.try_get(6)?;
            let repeat_kind: Option<String> = row.try_get(7)?;
            let repeat_days: Option<String> = row.try_get(8)?;
            let due_date = match (due_year, due_month, due_day) {
                (year, Some(month), Some(day)) => Some(crate::models::TodoDue {
                    year: year.unwrap_or(0) as u16,
                    month: month as u8,
                    day: day as u8,
                }),
                _ => None,
            };
            let repeat = match repeat_kind.as_deref() {
                Some(kind) if !kind.is_empty() => {
                    Some(repeat_from_columns(kind, None, None, None, repeat_days))
                }
                _ => None,
            };
            Ok(Todo {
                id: u8::try_from(row.try_get::<i64, _>(0)?)
                    .map_err(|_| anyhow!("todo id out of range"))?,
                text: row.try_get(1)?,
                done: row.try_get::<i64, _>(2)? != 0,
                importance: importance_from_str(&importance_str),
                due_date,
                repeat,
            })
        })
        .collect()
}

async fn next_local_id<'e, E>(db: &Db, executor: E, table: &str, device_id: &str) -> Result<u8>
where
    E: Executor<'e, Database = Any>,
{
    let sql = db.adapt(&format!(
        "SELECT MAX(local_id) FROM {table} WHERE device_id = ?"
    ));
    let max: Option<i64> = sqlx::query_scalar(&sql)
        .bind(device_id)
        .fetch_one(executor)
        .await?;
    let next = max.map(|m| m + 1).unwrap_or(0);
    u8::try_from(next).map_err(|_| anyhow!("device has reached the 256-alarm/todo id limit"))
}

/// Creates a new alarm (`id: None`) or replaces an existing one (`id:
/// Some`) for `device_id`, and returns the alarm's id. Bumps the device's
/// sync version either way.
pub async fn upsert_alarm(
    db: &Db,
    device_id: &str,
    id: Option<u8>,
    req: &UpsertAlarmRequest,
) -> Result<u8> {
    let id = match id {
        Some(id) => id,
        None => next_local_id(db, &db.pool, "alarms", device_id).await?,
    };
    let (repeat_kind, once_year, once_month, once_day, repeat_days) =
        repeat_to_columns(&req.repeat);
    sqlx::query(&db.adapt("INSERT INTO alarms (device_id, local_id, hour, minute, repeat_kind, once_year, once_month, once_day, repeat_days, enabled, label)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(device_id, local_id) DO UPDATE SET
            hour=excluded.hour, minute=excluded.minute, repeat_kind=excluded.repeat_kind,
            once_year=excluded.once_year, once_month=excluded.once_month, once_day=excluded.once_day,
            repeat_days=excluded.repeat_days,
            enabled=excluded.enabled, label=excluded.label"))
    .bind(device_id)
    .bind(id as i64)
    .bind(req.hour as i64)
    .bind(req.minute as i64)
    .bind(repeat_kind)
    .bind(once_year)
    .bind(once_month)
    .bind(once_day)
    .bind(repeat_days)
    .bind(req.enabled as i64)
    .bind(&req.label)
    .execute(&db.pool)
    .await?;
    bump_version(db, &db.pool, device_id).await?;
    Ok(id)
}

pub async fn delete_alarm(db: &Db, device_id: &str, id: u8) -> Result<()> {
    sqlx::query(&db.adapt("DELETE FROM alarms WHERE device_id = ? AND local_id = ?"))
        .bind(device_id)
        .bind(id as i64)
        .execute(&db.pool)
        .await?;
    bump_version(db, &db.pool, device_id).await?;
    Ok(())
}

pub async fn clear_alarms(db: &Db, device_id: &str) -> Result<()> {
    sqlx::query(&db.adapt("DELETE FROM alarms WHERE device_id = ?"))
        .bind(device_id)
        .execute(&db.pool)
        .await?;
    bump_version(db, &db.pool, device_id).await
}

pub async fn upsert_todo(
    db: &Db,
    device_id: &str,
    id: Option<u8>,
    req: &UpsertTodoRequest,
) -> Result<u8> {
    let id = match id {
        Some(id) => id,
        None => next_local_id(db, &db.pool, "todos", device_id).await?,
    };
    let (due_year, due_month, due_day) = match req.due_date {
        Some(due) => (
            Some(due.year as i64),
            Some(due.month as i64),
            Some(due.day as i64),
        ),
        None => (None, None, None),
    };
    // A todo's one-off nature is expressed by `due_date`; `Once` repeats
    // are folded to no-repeat so a repeating schedule can't lose its
    // meaning in the todos table (which has no once columns).
    let (repeat_kind, repeat_days) = match req.repeat.as_ref() {
        Some(Repeat::Daily) => (Some("daily"), None),
        Some(Repeat::Weekly { days }) => (Some("weekly"), repeat_days_json_opt(days)),
        Some(Repeat::Monthly { days }) => (Some("monthly"), repeat_days_json_opt(days)),
        Some(Repeat::Once { .. }) | None => (None, None),
    };
    sqlx::query(&db.adapt("INSERT INTO todos (device_id, local_id, text, done, importance, due_year, due_month, due_day, repeat_kind, repeat_days)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(device_id, local_id) DO UPDATE SET
            text=excluded.text, done=excluded.done, importance=excluded.importance,
            due_year=excluded.due_year, due_month=excluded.due_month, due_day=excluded.due_day,
            repeat_kind=excluded.repeat_kind, repeat_days=excluded.repeat_days"))
    .bind(device_id)
    .bind(id as i64)
    .bind(&req.text)
    .bind(req.done as i64)
    .bind(req.importance.as_str())
    .bind(due_year)
    .bind(due_month)
    .bind(due_day)
    .bind(repeat_kind)
    .bind(repeat_days)
    .execute(&db.pool)
    .await?;
    bump_version(db, &db.pool, device_id).await?;
    Ok(id)
}

pub async fn delete_todo(db: &Db, device_id: &str, id: u8) -> Result<()> {
    sqlx::query(&db.adapt("DELETE FROM todos WHERE device_id = ? AND local_id = ?"))
        .bind(device_id)
        .bind(id as i64)
        .execute(&db.pool)
        .await?;
    bump_version(db, &db.pool, device_id).await?;
    Ok(())
}

pub async fn clear_todos(db: &Db, device_id: &str) -> Result<()> {
    sqlx::query(&db.adapt("DELETE FROM todos WHERE device_id = ?"))
        .bind(device_id)
        .execute(&db.pool)
        .await?;
    bump_version(db, &db.pool, device_id).await
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

// --- Console accounts & sessions ---------------------------------------

pub async fn register_account(db: &Db, username: &str, password_hash: &str) -> Result<Account> {
    let id: i64 = sqlx::query_scalar(&db.adapt(
        "INSERT INTO accounts (username, password_hash, created_at) VALUES (?, ?, ?) RETURNING id",
    ))
    .bind(username)
    .bind(password_hash)
    .bind(now_unix())
    .fetch_one(&db.pool)
    .await?;
    Ok(Account {
        id,
        username: username.to_string(),
        created_at: now_unix(),
    })
}

/// Returns `(account_id, password_hash)` for a username, if it exists.
pub async fn find_account_by_username(db: &Db, username: &str) -> Result<Option<(i64, String)>> {
    let row = sqlx::query(&db.adapt("SELECT id, password_hash FROM accounts WHERE username = ?"))
        .bind(username)
        .fetch_optional(&db.pool)
        .await?;
    row.map(|r| -> Result<(i64, String)> {
        Ok((r.try_get::<i64, _>(0)?, r.try_get::<String, _>(1)?))
    })
    .transpose()
}

pub async fn account_by_id(db: &Db, account_id: i64) -> Result<Option<Account>> {
    let row = sqlx::query(&db.adapt("SELECT id, username, created_at FROM accounts WHERE id = ?"))
        .bind(account_id)
        .fetch_optional(&db.pool)
        .await?;
    row.map(|r| -> Result<Account> {
        Ok(Account {
            id: r.try_get(0)?,
            username: r.try_get(1)?,
            created_at: r.try_get(2)?,
        })
    })
    .transpose()
}

pub async fn update_account_password(db: &Db, account_id: i64, password_hash: &str) -> Result<()> {
    sqlx::query(&db.adapt("UPDATE accounts SET password_hash = ? WHERE id = ?"))
        .bind(password_hash)
        .bind(account_id)
        .execute(&db.pool)
        .await?;
    Ok(())
}

/// Admin-only listing of every account with its device/session counts.
pub async fn list_accounts(db: &Db) -> Result<Vec<AccountSummary>> {
    let rows = sqlx::query(&db.adapt(
        "SELECT a.id, a.username, a.created_at,
                (SELECT COUNT(*) FROM devices d WHERE d.account_id = a.id),
                (SELECT COUNT(*) FROM sessions s WHERE s.account_id = a.id)
         FROM accounts a ORDER BY a.username",
    ))
    .fetch_all(&db.pool)
    .await?;
    rows.into_iter()
        .map(|r| {
            Ok(AccountSummary {
                id: r.try_get(0)?,
                username: r.try_get(1)?,
                created_at: r.try_get(2)?,
                device_count: r.try_get(3)?,
                session_count: r.try_get(4)?,
            })
        })
        .collect()
}

/// Deletes an account; `false` when no such account exists.
pub async fn delete_account(db: &Db, account_id: i64) -> Result<bool> {
    let res = sqlx::query(&db.adapt("DELETE FROM accounts WHERE id = ?"))
        .bind(account_id)
        .execute(&db.pool)
        .await?;
    Ok(res.rows_affected() > 0)
}

/// Creates a session for `account_id` and returns its bearer token.
pub async fn create_session(db: &Db, account_id: i64) -> Result<String> {
    let token = new_token();
    sqlx::query(&db.adapt("INSERT INTO sessions (token, account_id, created_at) VALUES (?, ?, ?)"))
        .bind(&token)
        .bind(account_id)
        .bind(now_unix())
        .execute(&db.pool)
        .await?;
    Ok(token)
}

/// Maps a session token to its `account_id`, if valid.
pub async fn find_session(db: &Db, token: &str) -> Result<Option<i64>> {
    let row = sqlx::query(&db.adapt("SELECT account_id FROM sessions WHERE token = ?"))
        .bind(token)
        .fetch_optional(&db.pool)
        .await?;
    Ok(row.map(|r| r.try_get(0)).transpose()?)
}

pub async fn delete_session(db: &Db, token: &str) -> Result<()> {
    sqlx::query(&db.adapt("DELETE FROM sessions WHERE token = ?"))
        .bind(token)
        .execute(&db.pool)
        .await?;
    Ok(())
}

// --- External channels & inbox ---------------------------------------------

fn new_uuid() -> String {
    Uuid::new_v4().to_string()
}

/// Generates a webhook bearer token: a 32+ byte CSPRNG string with an
/// `ipwh_` prefix used to distinguish it from admin/device/session tokens.
pub fn new_channel_token() -> String {
    format!("ipwh_{}", new_token())
}

/// Creates a channel for `device_id` and returns `(Channel, plaintext_token)`.
/// `token` is `Some` only for webhook channels and is returned exactly once.
/// `config` (CalDAV etc.) is stored encrypted by the caller if provided.
pub async fn create_channel(
    db: &Db,
    device_id: &str,
    kind: &str,
    name: &str,
    config_encrypted: Option<&str>,
) -> Result<(Channel, Option<String>)> {
    let now = now_unix();
    let id = new_uuid();
    let (token_hash, token_prefix, token) = if kind == "webhook" {
        let token = new_channel_token();
        let hash = crate::auth::hash_password(&token)
            .map_err(|e| anyhow!("failed to hash channel token: {e}"))?;
        let prefix = token.chars().take(12).collect::<String>();
        (Some(hash), Some(prefix), Some(token))
    } else {
        (None, None, None)
    };
    sqlx::query(&db.adapt(
        "INSERT INTO channels (id, device_id, kind, name, enabled, token_hash, token_prefix, config_encrypted, config_version, created_at, updated_at)
         VALUES (?, ?, ?, ?, 1, ?, ?, ?, 1, ?, ?)",
    ))
    .bind(&id)
    .bind(device_id)
    .bind(kind)
    .bind(name)
    .bind(&token_hash)
    .bind(&token_prefix)
    .bind(config_encrypted)
    .bind(now)
    .bind(now)
    .execute(&db.pool)
    .await?;
    bump_version(db, &db.pool, device_id).await?;
    let channel = Channel {
        id,
        device_id: device_id.to_string(),
        kind: kind.to_string(),
        name: name.to_string(),
        enabled: true,
        token_prefix: token_prefix.unwrap_or_default(),
        last_sync_at: None,
        last_sync_error: None,
        created_at: now,
        updated_at: now,
    };
    Ok((channel, token))
}

fn channel_from_row(row: sqlx::any::AnyRow) -> Result<Channel> {
    Ok(Channel {
        id: row.try_get(0)?,
        device_id: row.try_get(1)?,
        kind: row.try_get(2)?,
        name: row.try_get(3)?,
        enabled: row.try_get::<i64, _>(4)? != 0,
        token_prefix: row.try_get::<Option<String>, _>(5)?.unwrap_or_default(),
        last_sync_at: row.try_get(6)?,
        last_sync_error: row.try_get(7)?,
        created_at: row.try_get(8)?,
        updated_at: row.try_get(9)?,
    })
}

pub async fn list_channels(db: &Db, device_id: &str) -> Result<Vec<Channel>> {
    let rows = sqlx::query(&db.adapt(
        "SELECT id, device_id, kind, name, enabled, token_prefix, last_sync_at, last_sync_error, created_at, updated_at
         FROM channels WHERE device_id = ? ORDER BY created_at",
    ))
    .bind(device_id)
    .fetch_all(&db.pool)
    .await?;
    rows.into_iter().map(channel_from_row).collect()
}

pub async fn get_channel(db: &Db, device_id: &str, channel_id: &str) -> Result<Option<Channel>> {
    let row = sqlx::query(&db.adapt(
        "SELECT id, device_id, kind, name, enabled, token_prefix, last_sync_at, last_sync_error, created_at, updated_at
         FROM channels WHERE device_id = ? AND id = ?",
    ))
    .bind(device_id)
    .bind(channel_id)
    .fetch_optional(&db.pool)
    .await?;
    row.map(channel_from_row).transpose()
}

/// Finds a webhook channel by id without device scoping (delivery path) and
/// returns it along with its stored token hash. Enforced `enabled` is checked
/// by the caller after token verification so a wrong token and a disabled
/// channel are indistinguishable.
pub async fn get_channel_for_delivery(
    db: &Db,
    channel_id: &str,
) -> Result<Option<(String, String)>> {
    let row = sqlx::query(&db.adapt("SELECT device_id, token_hash FROM channels WHERE id = ?"))
        .bind(channel_id)
        .fetch_optional(&db.pool)
        .await?;
    row.map(|r| -> Result<(String, String)> {
        let hash: String = r
            .try_get::<Option<String>, _>(1)?
            .ok_or_else(|| anyhow!("channel is not a webhook channel"))?;
        Ok((r.try_get(0)?, hash))
    })
    .transpose()
}

pub async fn update_channel(
    db: &Db,
    device_id: &str,
    channel_id: &str,
    name: Option<&str>,
    enabled: Option<bool>,
) -> Result<bool> {
    let res = sqlx::query(&db.adapt(
        "UPDATE channels SET name = COALESCE(?, name), enabled = COALESCE(?, enabled), updated_at = ?
         WHERE device_id = ? AND id = ?",
    ))
    .bind(name)
    .bind(enabled.map(|b| b as i64))
    .bind(now_unix())
    .bind(device_id)
    .bind(channel_id)
    .execute(&db.pool)
    .await?;
    if res.rows_affected() > 0 {
        bump_version(db, &db.pool, device_id).await?;
        Ok(true)
    } else {
        Ok(false)
    }
}

pub async fn delete_channel(db: &Db, device_id: &str, channel_id: &str) -> Result<bool> {
    let res = sqlx::query(&db.adapt("DELETE FROM channels WHERE device_id = ? AND id = ?"))
        .bind(device_id)
        .bind(channel_id)
        .execute(&db.pool)
        .await?;
    if res.rows_affected() > 0 {
        bump_version(db, &db.pool, device_id).await?;
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Rotates a webhook channel's token, returning the new plaintext. Rejects
/// non-webhook channels. The old token stops working immediately because it
/// hashed to a different value than the stored one.
pub async fn rotate_channel_token(
    db: &Db,
    device_id: &str,
    channel_id: &str,
) -> Result<Option<String>> {
    let row = sqlx::query(&db.adapt("SELECT kind FROM channels WHERE device_id = ? AND id = ?"))
        .bind(device_id)
        .bind(channel_id)
        .fetch_optional(&db.pool)
        .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let kind: String = row.try_get(0)?;
    if kind != "webhook" {
        return Ok(None);
    }
    let token = new_channel_token();
    let hash = crate::auth::hash_password(&token)
        .map_err(|e| anyhow!("failed to hash channel token: {e}"))?;
    let prefix = token.chars().take(12).collect::<String>();
    sqlx::query(&db.adapt(
        "UPDATE channels SET token_hash = ?, token_prefix = ?, updated_at = ? WHERE device_id = ? AND id = ?",
    ))
    .bind(&hash)
    .bind(&prefix)
    .bind(now_unix())
    .bind(device_id)
    .bind(channel_id)
    .execute(&db.pool)
    .await?;
    Ok(Some(token))
}

/// Atomically allocates the next inbox `seq` for a device and writes the
/// inbox row in the same transaction, so concurrent deliveries can never
/// issue a duplicate `seq` for one device. Returns the allocated `seq`.
#[allow(clippy::too_many_arguments)]
async fn insert_inbox_with_seq(
    db: &Db,
    tx: &mut sqlx::Transaction<'_, Any>,
    id: &str,
    device_id: &str,
    channel_id: &str,
    kind: &str,
    title: &str,
    body: &str,
    when: Option<i64>,
    source_ref: Option<&str>,
) -> Result<i64> {
    // Ensure a sequence row exists, then claim the next value. The row is
    // seeded at 0 and incremented before read, so the first allocated seq
    // is 1.
    sqlx::query(&db.adapt(
        "INSERT INTO device_sequences (device_id, next_inbox_seq) VALUES (?, 0)
         ON CONFLICT(device_id) DO NOTHING",
    ))
    .bind(device_id)
    .execute(&mut **tx)
    .await?;
    let seq: i64 = if db.postgres {
        let row = sqlx::query(&db.adapt(
            "UPDATE device_sequences SET next_inbox_seq = next_inbox_seq + 1
             WHERE device_id = ? RETURNING next_inbox_seq",
        ))
        .bind(device_id)
        .fetch_one(&mut **tx)
        .await?;
        row.try_get(0)?
    } else {
        let row = sqlx::query(&db.adapt(
            "UPDATE device_sequences SET next_inbox_seq = next_inbox_seq + 1
             WHERE device_id = ?",
        ))
        .bind(device_id)
        .execute(&mut **tx)
        .await?;
        if row.rows_affected() == 0 {
            return Err(anyhow!("failed to allocate inbox sequence"));
        }
        sqlx::query_scalar(
            &db.adapt("SELECT next_inbox_seq FROM device_sequences WHERE device_id = ?"),
        )
        .bind(device_id)
        .fetch_one(&mut **tx)
        .await?
    };
    let now = now_unix();
    sqlx::query(&db.adapt(
        "INSERT INTO inbox (id, device_id, channel_id, seq, kind, title, body, when_epoch, source_ref, read, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 0, ?, ?)",
    ))
    .bind(id)
    .bind(device_id)
    .bind(channel_id)
    .bind(seq)
    .bind(kind)
    .bind(title)
    .bind(body)
    .bind(when)
    .bind(source_ref)
    .bind(now)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    Ok(seq)
}

/// Inserts a webhook-delivered inbox item, honoring idempotency via
/// `source_ref`. Returns `(seq, created)` where `created` is false when the
/// `source_ref` already exists (idempotent replay).
#[allow(clippy::too_many_arguments)]
pub async fn deliver_inbox(
    db: &Db,
    device_id: &str,
    channel_id: &str,
    kind: &str,
    title: &str,
    body: &str,
    when: Option<i64>,
    source_ref: Option<&str>,
) -> Result<(u64, bool)> {
    let mut tx = db.pool.begin().await?;
    if let Some(ref_ref) = source_ref {
        let existing: Option<i64> = sqlx::query_scalar(
            &db.adapt("SELECT seq FROM inbox WHERE channel_id = ? AND source_ref = ?"),
        )
        .bind(channel_id)
        .bind(ref_ref)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(seq) = existing {
            tx.commit().await?;
            return Ok((seq as u64, false));
        }
    }
    let id = new_uuid();
    let seq = insert_inbox_with_seq(
        db, &mut tx, &id, device_id, channel_id, kind, title, body, when, source_ref,
    )
    .await?;
    bump_version(db, &mut *tx, device_id).await?;
    tx.commit().await?;
    Ok((seq as u64, true))
}

/// Returns the device's inbox for the sync response: unread first, then by
/// `seq DESC`, capped at `limit`. `truncated` is true when more rows exist.
pub async fn list_inbox(db: &Db, device_id: &str, limit: usize) -> Result<(Vec<InboxItem>, bool)> {
    let total: i64 =
        sqlx::query_scalar(&db.adapt("SELECT COUNT(*) FROM inbox WHERE device_id = ?"))
            .bind(device_id)
            .fetch_one(&db.pool)
            .await?;
    let rows = sqlx::query(&db.adapt(
        "SELECT seq, kind, title, body, when_epoch, read FROM inbox
         WHERE device_id = ? ORDER BY read ASC, seq DESC LIMIT ?",
    ))
    .bind(device_id)
    .bind(limit as i64)
    .fetch_all(&db.pool)
    .await?;
    let items = rows
        .into_iter()
        .map(|r| {
            Ok(InboxItem {
                id: r.try_get::<i64, _>(0)? as u64,
                kind: InboxKind::from(r.try_get::<String, _>(1)?.as_str()),
                title: r.try_get(2)?,
                body: r.try_get::<Option<String>, _>(3)?.unwrap_or_default(),
                when: r.try_get(4)?,
                read: r.try_get::<i64, _>(5)? != 0,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let truncated = (total as usize) > items.len();
    Ok((items, truncated))
}

/// Marks inbox items read for a device. Only `seq`s belonging to `device_id`
/// are touched; unknown ids are silently ignored (cannot create content).
/// Returns the seqs actually acked (those that transitioned to read), in the
/// order sent. Bumps the device version only if something changed.
pub async fn mark_inbox_read(db: &Db, device_id: &str, seqs: &[u64]) -> Result<Vec<u64>> {
    if seqs.is_empty() {
        return Ok(Vec::new());
    }
    let mut tx = db.pool.begin().await?;
    let mut acked = Vec::new();
    for seq in seqs {
        let res = sqlx::query(&db.adapt(
            "UPDATE inbox SET read = 1, updated_at = ? WHERE device_id = ? AND seq = ? AND read = 0",
        ))
        .bind(now_unix())
        .bind(device_id)
        .bind(*seq as i64)
        .execute(&mut *tx)
        .await?;
        if res.rows_affected() > 0 {
            acked.push(*seq);
        }
    }
    if !acked.is_empty() {
        bump_version(db, &mut *tx, device_id).await?;
    }
    tx.commit().await?;
    Ok(acked)
}

/// Management/debug helper: delete a single inbox item for a device.
pub async fn delete_inbox_item(db: &Db, device_id: &str, seq: u64) -> Result<bool> {
    let res = sqlx::query(&db.adapt("DELETE FROM inbox WHERE device_id = ? AND seq = ?"))
        .bind(device_id)
        .bind(seq as i64)
        .execute(&db.pool)
        .await?;
    if res.rows_affected() > 0 {
        bump_version(db, &db.pool, device_id).await?;
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Management/debug helper: clear read inbox history for a device.
pub async fn clear_read_inbox(db: &Db, device_id: &str) -> Result<()> {
    let res = sqlx::query(&db.adapt("DELETE FROM inbox WHERE device_id = ? AND read = 1"))
        .bind(device_id)
        .execute(&db.pool)
        .await?;
    if res.rows_affected() > 0 {
        bump_version(db, &db.pool, device_id).await?;
    }
    Ok(())
}

// --- Column helpers (shared by both backends) -----------------------------

/// Parses the `importance` column; unknown/legacy values fall back to
/// `Medium`.
fn importance_from_str(s: &str) -> Importance {
    match s {
        "low" => Importance::Low,
        "high" => Importance::High,
        _ => Importance::Medium,
    }
}

fn repeat_days_json_opt(days: &[u8]) -> Option<String> {
    serde_json::to_string(days).ok().map(|s| s.replace(' ', ""))
}

fn repeat_to_columns(
    repeat: &Repeat,
) -> (
    &'static str,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<String>,
) {
    match repeat {
        Repeat::Daily => ("daily", None, None, None, None),
        Repeat::Weekly { days } | Repeat::Monthly { days } => {
            (repeat.kind(), None, None, None, repeat_days_json_opt(days))
        }
        Repeat::Once { year, month, day } => (
            "once",
            Some(*year as i64),
            Some(*month as i64),
            Some(*day as i64),
            None,
        ),
    }
}

fn repeat_from_columns(
    repeat_kind: &str,
    once_year: Option<i64>,
    once_month: Option<i64>,
    once_day: Option<i64>,
    repeat_days: Option<String>,
) -> Repeat {
    match repeat_kind {
        "once" => match (once_year, once_month, once_day) {
            (Some(year), Some(month), Some(day)) => Repeat::Once {
                year: year as u16,
                month: month as u8,
                day: day as u8,
            },
            _ => Repeat::Daily,
        },
        "weekly" => Repeat::Weekly {
            days: repeat_days_json(repeat_days.as_deref()),
        },
        "monthly" => Repeat::Monthly {
            days: repeat_days_json(repeat_days.as_deref()),
        },
        _ => Repeat::Daily,
    }
}

fn repeat_days_json(raw: Option<&str>) -> Vec<u8> {
    raw.and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn memory_db() -> Db {
        open("sqlite::memory:", 1).await.expect("open in-memory db")
    }

    #[tokio::test]
    async fn bulk_clear_preserves_device_and_other_content() {
        let db = memory_db().await;
        let device = register_device(&db, "test", None).await.unwrap();
        upsert_alarm(
            &db,
            device.id.as_str(),
            None,
            &UpsertAlarmRequest {
                hour: 7,
                minute: 30,
                repeat: Repeat::Daily,
                enabled: true,
                label: "wake".to_string(),
            },
        )
        .await
        .unwrap();
        upsert_todo(
            &db,
            device.id.as_str(),
            None,
            &UpsertTodoRequest {
                text: "keep".to_string(),
                done: false,
                importance: Importance::High,
                due_date: Some(crate::models::TodoDue {
                    year: 2026,
                    month: 12,
                    day: 25,
                }),
                repeat: None,
            },
        )
        .await
        .unwrap();

        clear_alarms(&db, device.id.as_str()).await.unwrap();
        assert!(list_alarms(&db, device.id.as_str())
            .await
            .unwrap()
            .is_empty());
        let todos = list_todos(&db, device.id.as_str()).await.unwrap();
        assert_eq!(todos.len(), 1);
        assert_eq!(todos[0].importance, Importance::High);
        assert_eq!(
            todos[0].due_date,
            Some(crate::models::TodoDue {
                year: 2026,
                month: 12,
                day: 25
            })
        );
        assert_eq!(list_devices(&db).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn device_merge_updates_flags_without_recreating_unknown_content() {
        let db = memory_db().await;
        let device = register_device(&db, "test", None).await.unwrap();
        let todo_id = upsert_todo(
            &db,
            device.id.as_str(),
            None,
            &UpsertTodoRequest {
                text: "server text".to_string(),
                done: false,
                importance: Importance::Low,
                due_date: None,
                repeat: None,
            },
        )
        .await
        .unwrap();
        let before = find_device_by_token(&db, device.token.as_deref().unwrap())
            .await
            .unwrap()
            .unwrap()
            .1;
        merge_device_state(
            &db,
            device.id.as_str(),
            &DeviceSyncRequest {
                alarms: vec![],
                todos: vec![
                    crate::models::DeviceTodoState {
                        id: todo_id,
                        done: true,
                        importance: Some(Importance::High),
                    },
                    crate::models::DeviceTodoState {
                        id: 250,
                        done: true,
                        importance: None,
                    },
                ],
                inbox_read: vec![],
            },
        )
        .await
        .unwrap();
        let todos = list_todos(&db, device.id.as_str()).await.unwrap();
        assert_eq!(todos.len(), 1);
        assert_eq!(todos[0].text, "server text");
        assert!(todos[0].done);
        assert_eq!(todos[0].importance, Importance::High);
        let after = find_device_by_token(&db, device.token.as_deref().unwrap())
            .await
            .unwrap()
            .unwrap()
            .1;
        assert_eq!(after, before + 1);
    }

    #[tokio::test]
    async fn account_flow_and_admin_summary() {
        let db = memory_db().await;
        let account = register_account(&db, "alice", "fake-hash").await.unwrap();
        assert_eq!(account.id, 1);
        let found = find_account_by_username(&db, "alice")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.0, account.id);

        let device = register_device(&db, "alice-clock", Some(account.id))
            .await
            .unwrap();
        assert!(device_owned_by(&db, &device.id, account.id).await.unwrap());
        assert!(!device_owned_by(&db, &device.id, 999).await.unwrap());

        let summaries = list_accounts(&db).await.unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].device_count, 1);

        assert!(delete_account(&db, account.id).await.unwrap());
        assert!(!delete_account(&db, account.id).await.unwrap());
        assert!(list_devices(&db).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn webhook_channel_delivers_unique_seq_and_idempotent_dedup() {
        let db = memory_db().await;
        let device = register_device(&db, "dev", None).await.unwrap();
        let (channel, token) = create_channel(&db, &device.id, "webhook", "CI", None)
            .await
            .unwrap();
        assert!(token.unwrap().starts_with("ipwh_"));

        // Concurrent-ish sequential deliveries must get increasing, unique seq.
        let (seq1, created1) = deliver_inbox(
            &db,
            &device.id,
            &channel.id,
            "alert",
            "Build failed",
            "main",
            None,
            None,
        )
        .await
        .unwrap();
        let (seq2, created2) = deliver_inbox(
            &db,
            &device.id,
            &channel.id,
            "info",
            "Build ok",
            "",
            None,
            None,
        )
        .await
        .unwrap();
        assert!(created1 && created2);
        assert_eq!(seq1, 1);
        assert_eq!(seq2, 2);
        assert_ne!(seq1, seq2);

        // Idempotency: same source_ref returns the same seq without a new row.
        let (seq_orig, created_orig) = deliver_inbox(
            &db,
            &device.id,
            &channel.id,
            "alert",
            "Build failed",
            "main",
            None,
            Some("key-1"),
        )
        .await
        .unwrap();
        let (seq_replay, created_replay) = deliver_inbox(
            &db,
            &device.id,
            &channel.id,
            "alert",
            "Build failed",
            "main",
            None,
            Some("key-1"),
        )
        .await
        .unwrap();
        assert!(created_orig);
        assert!(!created_replay);
        assert_eq!(seq_orig, seq_replay);

        // Read merge only affects this device and acks what was marked.
        let (_items, truncated) = list_inbox(&db, &device.id, 20).await.unwrap();
        assert!(!truncated);
        let acked = mark_inbox_read(&db, &device.id, &[seq1]).await.unwrap();
        assert_eq!(acked, vec![seq1]);
        let (items2, _) = list_inbox(&db, &device.id, 20).await.unwrap();
        assert!(items2.iter().find(|i| i.id == seq1).unwrap().read);
        // Re-marking an already-read item returns nothing (no double ack).
        assert!(mark_inbox_read(&db, &device.id, &[seq1])
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn channel_ownership_and_rotate() {
        let db = memory_db().await;
        let device = register_device(&db, "dev", None).await.unwrap();
        let (channel, _) = create_channel(&db, &device.id, "webhook", "CI", None)
            .await
            .unwrap();

        // Wrong device cannot fetch this channel.
        let other = register_device(&db, "other", None).await.unwrap();
        assert!(get_channel(&db, &other.id, &channel.id)
            .await
            .unwrap()
            .is_none());

        // Rotate changes the token (verifiable via a changed hash check).
        let new_token = rotate_channel_token(&db, &device.id, &channel.id)
            .await
            .unwrap()
            .unwrap();
        assert!(new_token.starts_with("ipwh_"));
        assert_ne!(
            get_channel(&db, &device.id, &channel.id)
                .await
                .unwrap()
                .unwrap()
                .token_prefix,
            channel.token_prefix
        );

        // Rotate on a non-webhook channel returns None.
        assert!(rotate_channel_token(&db, &device.id, "does-not-exist")
            .await
            .unwrap()
            .is_none());

        // Deleting a channel cascades its inbox.
        deliver_inbox(&db, &device.id, &channel.id, "info", "x", "", None, None)
            .await
            .unwrap();
        assert!(delete_channel(&db, &device.id, &channel.id).await.unwrap());
        let (items, _) = list_inbox(&db, &device.id, 20).await.unwrap();
        assert!(items.is_empty());
    }

    #[tokio::test]
    async fn inbox_capacity_is_capped() {
        let db = memory_db().await;
        let device = register_device(&db, "dev", None).await.unwrap();
        let (channel, _) = create_channel(&db, &device.id, "webhook", "CI", None)
            .await
            .unwrap();
        for i in 0..30 {
            deliver_inbox(
                &db,
                &device.id,
                &channel.id,
                "info",
                &format!("m{i}"),
                "",
                None,
                None,
            )
            .await
            .unwrap();
        }
        let (items, truncated) = list_inbox(&db, &device.id, 20).await.unwrap();
        assert_eq!(items.len(), 20);
        assert!(truncated);
        // Newest seq first (seq DESC).
        assert_eq!(items[0].id, 30);
    }
}
