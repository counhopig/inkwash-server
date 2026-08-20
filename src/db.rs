//! SQLite storage. A single `Arc<Mutex<Connection>>` is shared across axum
//! handlers rather than a connection pool or async driver - this is a
//! personal-scale server (one owner, a handful of devices), request volume
//! is trivially low, and `rusqlite` calls are fast enough that blocking the
//! handler task briefly is a fine trade for not pulling in `sqlx`/deadpool
//! and its offline-query-cache setup for a project this size.

use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Context, Result};
use rand::distributions::Alphanumeric;
use rand::Rng;
use rusqlite::{params, Connection};
use uuid::Uuid;

use crate::models::{
    Account, AccountSummary, Alarm, Device, DeviceSyncRequest, Importance, Repeat, Todo,
    UpsertAlarmRequest, UpsertTodoRequest,
};

pub type Db = Arc<Mutex<Connection>>;

const DEVICES_TABLE: &str = "
    CREATE TABLE IF NOT EXISTS devices (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        token TEXT NOT NULL UNIQUE,
        version INTEGER NOT NULL DEFAULT 0,
        created_at INTEGER NOT NULL,
        account_id INTEGER REFERENCES accounts(id) ON DELETE CASCADE
    )";
const ALARMS_TABLE: &str = "
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
    )";
const TODOS_TABLE: &str = "
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
    )";
const ACCOUNTS_TABLE: &str = "
    CREATE TABLE IF NOT EXISTS accounts (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        username TEXT NOT NULL UNIQUE,
        password_hash TEXT NOT NULL,
        created_at INTEGER NOT NULL
    )";
const SESSIONS_TABLE: &str = "
    CREATE TABLE IF NOT EXISTS sessions (
        token TEXT PRIMARY KEY,
        account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
        created_at INTEGER NOT NULL
    )";

pub fn open(path: &str) -> Result<Db> {
    let mut conn = Connection::open(path).with_context(|| format!("failed to open {path}"))?;
    conn.pragma_update(None, "foreign_keys", true)?;
    conn.execute_batch(&format!(
        "{ACCOUNTS_TABLE}; {SESSIONS_TABLE}; {DEVICES_TABLE}; {ALARMS_TABLE}; {TODOS_TABLE};"
    ))?;
    migrate_legacy_integer_ids(&mut conn)?;
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
        let _ = conn.execute(stmt, []);
    }
    Ok(Arc::new(Mutex::new(conn)))
}

/// Legacy pre-UUID `alarms` row shape used only by the migration snapshot.
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

/// Migrates databases from the pre-UUID era where `devices.id` was an
/// `INTEGER PRIMARY KEY AUTOINCREMENT` and `alarms`/`todos` referenced it
/// with an INTEGER `device_id`. Personal-scale data (a handful of devices,
/// each with a handful of records) is copied over row by row; the device
/// auth token is preserved, so devices keep syncing without re-registering.
fn migrate_legacy_integer_ids(conn: &mut Connection) -> Result<()> {
    let legacy: i64 = conn.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('devices') WHERE name = 'id' AND type = 'INTEGER'",
        [],
        |row| row.get(0),
    )?;
    if legacy == 0 {
        return Ok(());
    }
    tracing::info!("detected pre-UUID device ids; migrating to UUID keys");

    // Snapshot the legacy rows, then rename the old tables out of the way.
    let devices: Vec<(i64, String, String, i64, i64)> = conn
        .prepare("SELECT id, name, token, version, created_at FROM devices")?
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let alarms: Vec<LegacyAlarmRow> =
        conn.prepare(
            "SELECT device_id, local_id, hour, minute, repeat_kind, once_year, once_month, once_day, enabled, label FROM alarms",
        )?
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
                row.get(7)?,
                row.get(8)?,
                row.get(9)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let todos: Vec<(i64, i64, String, i64)> = conn
        .prepare("SELECT device_id, local_id, text, done FROM todos")?
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    conn.execute_batch(&format!(
        "DROP TABLE alarms; DROP TABLE todos; DROP TABLE devices;
         {DEVICES_TABLE}; {ALARMS_TABLE}; {TODOS_TABLE};"
    ))?;

    let mut id_map = std::collections::HashMap::new();
    for (old_id, name, token, version, created_at) in &devices {
        let new_id = Uuid::new_v4().to_string();
        id_map.insert(*old_id, new_id.clone());
        conn.execute(
            "INSERT INTO devices (id, name, token, version, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![new_id, name, token, version, created_at],
        )?;
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
        conn.execute(
            "INSERT INTO alarms (device_id, local_id, hour, minute, repeat_kind, once_year, once_month, once_day, enabled, label)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                new_device_id,
                local_id,
                hour,
                minute,
                repeat_kind,
                once_year,
                once_month,
                once_day,
                enabled,
                label
            ],
        )?;
    }
    for (device_id, local_id, text, done) in &todos {
        let Some(new_device_id) = id_map.get(device_id) else {
            continue;
        };
        conn.execute(
            "INSERT INTO todos (device_id, local_id, text, done) VALUES (?1, ?2, ?3, ?4)",
            params![new_device_id, local_id, text, done],
        )?;
    }
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

/// Registers a new device and returns it with its token populated. The
/// token is a bearer credential (see `docs/sync-api.md`'s Security
/// section in the firmware repo) - only ever returned here, at creation
/// time; `list_devices` omits it, so losing it means re-registering.
/// `account_id: None` creates an unowned device (only reachable with the
/// `ADMIN_TOKEN`); `Some` ties the device to a console account.
pub fn register_device(db: &Db, name: &str, account_id: Option<i64>) -> Result<Device> {
    let conn = db.lock().unwrap();
    let token = new_token();
    let now = now_unix();
    let id = Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO devices (id, name, token, version, created_at, account_id) VALUES (?1, ?2, ?3, 0, ?4, ?5)",
        params![id, name, token, now, account_id],
    )?;
    Ok(Device {
        id,
        name: name.to_string(),
        token: Some(token),
    })
}

pub fn list_devices(db: &Db) -> Result<Vec<Device>> {
    let conn = db.lock().unwrap();
    let mut stmt = conn.prepare("SELECT id, name FROM devices ORDER BY name")?;
    let rows = stmt
        .query_map([], |row| {
            Ok(Device {
                id: row.get(0)?,
                name: row.get(1)?,
                token: None,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Devices owned by one console account (for `GET /api/devices` when the
/// caller is an account session rather than the admin token).
pub fn list_account_devices(db: &Db, account_id: i64) -> Result<Vec<Device>> {
    let conn = db.lock().unwrap();
    let mut stmt =
        conn.prepare("SELECT id, name FROM devices WHERE account_id = ?1 ORDER BY name")?;
    let rows = stmt
        .query_map(params![account_id], |row| {
            Ok(Device {
                id: row.get(0)?,
                name: row.get(1)?,
                token: None,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Whether `device_id` exists and is owned by `account_id`. Used to scope
/// alarm/todo routes to an account's own devices.
pub fn device_owned_by(db: &Db, device_id: &str, account_id: i64) -> Result<bool> {
    let conn = db.lock().unwrap();
    conn.query_row(
        "SELECT COUNT(*) FROM devices WHERE id = ?1 AND account_id = ?2",
        params![device_id, account_id],
        |row| row.get::<_, i64>(0),
    )
    .map(|count| count > 0)
    .map_err(Into::into)
}

pub fn delete_device(db: &Db, device_id: &str) -> Result<()> {
    let conn = db.lock().unwrap();
    conn.execute("DELETE FROM devices WHERE id = ?1", params![device_id])?;
    Ok(())
}

/// Looks up the device owning `token`, returning `(device_id, version)`.
/// `version` becomes the ETag for `GET /api/sync` - see
/// `routes::device_sync`.
pub fn find_device_by_token(db: &Db, token: &str) -> Result<Option<(String, i64)>> {
    let conn = db.lock().unwrap();
    conn.query_row(
        "SELECT id, version FROM devices WHERE token = ?1",
        params![token],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .map(Some)
    .or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        e => Err(e.into()),
    })
}

// --- Console accounts & sessions ---------------------------------------

pub fn register_account(db: &Db, username: &str, password_hash: &str) -> Result<Account> {
    let conn = db.lock().unwrap();
    conn.execute(
        "INSERT INTO accounts (username, password_hash, created_at) VALUES (?1, ?2, ?3)",
        params![username, password_hash, now_unix()],
    )?;
    let id = conn.last_insert_rowid();
    Ok(Account {
        id,
        username: username.to_string(),
        created_at: now_unix(),
    })
}

/// Returns `(account_id, password_hash)` for a username, if it exists.
pub fn find_account_by_username(db: &Db, username: &str) -> Result<Option<(i64, String)>> {
    let conn = db.lock().unwrap();
    conn.query_row(
        "SELECT id, password_hash FROM accounts WHERE username = ?1",
        params![username],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .map(Some)
    .or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        e => Err(e.into()),
    })
}

pub fn account_by_id(db: &Db, account_id: i64) -> Result<Option<Account>> {
    let conn = db.lock().unwrap();
    conn.query_row(
        "SELECT id, username, created_at FROM accounts WHERE id = ?1",
        params![account_id],
        |row| {
            Ok(Account {
                id: row.get(0)?,
                username: row.get(1)?,
                created_at: row.get(2)?,
            })
        },
    )
    .map(Some)
    .or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        e => Err(e.into()),
    })
}

pub fn update_account_password(db: &Db, account_id: i64, password_hash: &str) -> Result<()> {
    let conn = db.lock().unwrap();
    conn.execute(
        "UPDATE accounts SET password_hash = ?2 WHERE id = ?1",
        params![account_id, password_hash],
    )?;
    Ok(())
}

/// Admin-only listing of every account with its device/session counts
/// (no password hash - see `AccountSummary`).
pub fn list_accounts(db: &Db) -> Result<Vec<AccountSummary>> {
    let conn = db.lock().unwrap();
    let mut stmt = conn.prepare(
        "SELECT a.id, a.username, a.created_at,
                (SELECT COUNT(*) FROM devices d WHERE d.account_id = a.id),
                (SELECT COUNT(*) FROM sessions s WHERE s.account_id = a.id)
         FROM accounts a ORDER BY a.username",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok(AccountSummary {
                id: row.get(0)?,
                username: row.get(1)?,
                created_at: row.get(2)?,
                device_count: row.get(3)?,
                session_count: row.get(4)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Deletes an account; `false` when no such account exists. Devices and
/// sessions cascade via `ON DELETE CASCADE`.
pub fn delete_account(db: &Db, account_id: i64) -> Result<bool> {
    let conn = db.lock().unwrap();
    let deleted = conn.execute("DELETE FROM accounts WHERE id = ?1", params![account_id])?;
    Ok(deleted > 0)
}

/// Creates a session for `account_id` and returns its bearer token.
/// Sessions are persistent (stored in the DB) so the console stays logged
/// in across server restarts; revoke by deleting the session.
pub fn create_session(db: &Db, account_id: i64) -> Result<String> {
    let conn = db.lock().unwrap();
    let token = new_token();
    conn.execute(
        "INSERT INTO sessions (token, account_id, created_at) VALUES (?1, ?2, ?3)",
        params![token, account_id, now_unix()],
    )?;
    Ok(token)
}

/// Maps a session token to its `account_id`, if valid.
pub fn find_session(db: &Db, token: &str) -> Result<Option<i64>> {
    let conn = db.lock().unwrap();
    conn.query_row(
        "SELECT account_id FROM sessions WHERE token = ?1",
        params![token],
        |row| row.get(0),
    )
    .map(Some)
    .or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        e => Err(e.into()),
    })
}

pub fn delete_session(db: &Db, token: &str) -> Result<()> {
    let conn = db.lock().unwrap();
    conn.execute("DELETE FROM sessions WHERE token = ?1", params![token])?;
    Ok(())
}

fn bump_version(conn: &Connection, device_id: &str) -> Result<()> {
    conn.execute(
        "UPDATE devices SET version = version + 1 WHERE id = ?1",
        params![device_id],
    )?;
    Ok(())
}

/// Applies only the fields the physical device is allowed to edit. Unknown
/// IDs are ignored so stale device caches cannot recreate content deleted by
/// Desktop/Server. Returns the resulting device version.
pub fn merge_device_state(db: &Db, device_id: &str, state: &DeviceSyncRequest) -> Result<i64> {
    let mut conn = db.lock().unwrap();
    let tx = conn.transaction()?;
    let mut changed = false;
    for alarm in &state.alarms {
        changed |= tx.execute(
            "UPDATE alarms SET enabled = ?3 WHERE device_id = ?1 AND local_id = ?2 AND enabled != ?3",
            params![device_id, alarm.id, alarm.enabled],
        )? > 0;
    }
    for todo in &state.todos {
        changed |= tx.execute(
            "UPDATE todos SET done = ?3 WHERE device_id = ?1 AND local_id = ?2 AND done != ?3",
            params![device_id, todo.id, todo.done],
        )? > 0;
        if let Some(importance) = todo.importance {
            changed |= tx.execute(
                "UPDATE todos SET importance = ?3 WHERE device_id = ?1 AND local_id = ?2 AND importance != ?3",
                params![device_id, todo.id, importance.as_str()],
            )? > 0;
        }
    }
    if changed {
        bump_version(&tx, device_id)?;
    }
    let version = tx.query_row(
        "SELECT version FROM devices WHERE id = ?1",
        params![device_id],
        |row| row.get(0),
    )?;
    tx.commit()?;
    Ok(version)
}

pub fn list_alarms(db: &Db, device_id: &str) -> Result<Vec<Alarm>> {
    let conn = db.lock().unwrap();
    let mut stmt = conn.prepare(
        "SELECT local_id, hour, minute, repeat_kind, once_year, once_month, once_day, repeat_days, enabled, label
         FROM alarms WHERE device_id = ?1 ORDER BY local_id",
    )?;
    let rows = stmt
        .query_map(params![device_id], |row| {
            let repeat_kind: String = row.get(3)?;
            let repeat = repeat_from_columns(
                &repeat_kind,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
                row.get(7)?,
            );
            Ok(Alarm {
                id: row.get(0)?,
                hour: row.get(1)?,
                minute: row.get(2)?,
                repeat,
                enabled: row.get::<_, i64>(8)? != 0,
                label: row.get(9)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn list_todos(db: &Db, device_id: &str) -> Result<Vec<Todo>> {
    let conn = db.lock().unwrap();
    let mut stmt = conn.prepare(
        "SELECT local_id, text, done, importance, due_year, due_month, due_day, repeat_kind, repeat_days
         FROM todos WHERE device_id = ?1 ORDER BY local_id",
    )?;
    let rows = stmt
        .query_map(params![device_id], |row| {
            let importance_str: String = row.get(3)?;
            let due_year: Option<i64> = row.get(4)?;
            let due_month: Option<i64> = row.get(5)?;
            let due_day: Option<i64> = row.get(6)?;
            let repeat_kind: Option<String> = row.get(7)?;
            let repeat_days: Option<String> = row.get(8)?;
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
                id: row.get(0)?,
                text: row.get(1)?,
                done: row.get::<_, i64>(2)? != 0,
                importance: importance_from_str(&importance_str),
                due_date,
                repeat,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn next_local_id(conn: &Connection, table: &str, device_id: &str) -> Result<u8> {
    let max: Option<i64> = conn.query_row(
        &format!("SELECT MAX(local_id) FROM {table} WHERE device_id = ?1"),
        params![device_id],
        |row| row.get(0),
    )?;
    let next = max.map(|m| m + 1).unwrap_or(0);
    u8::try_from(next).map_err(|_| anyhow!("device has reached the 256-alarm/todo id limit"))
}

/// Creates a new alarm (`id: None`) or replaces an existing one (`id:
/// Some`) for `device_id`, and returns the alarm's id. Bumps the device's
/// sync version either way.
pub fn upsert_alarm(
    db: &Db,
    device_id: &str,
    id: Option<u8>,
    req: &UpsertAlarmRequest,
) -> Result<u8> {
    let conn = db.lock().unwrap();
    let id = match id {
        Some(id) => id,
        None => next_local_id(&conn, "alarms", device_id)?,
    };
    let (repeat_kind, once_year, once_month, once_day, repeat_days) =
        repeat_to_columns(&req.repeat);
    conn.execute(
        "INSERT INTO alarms (device_id, local_id, hour, minute, repeat_kind, once_year, once_month, once_day, repeat_days, enabled, label)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
         ON CONFLICT(device_id, local_id) DO UPDATE SET
            hour=excluded.hour, minute=excluded.minute, repeat_kind=excluded.repeat_kind,
            once_year=excluded.once_year, once_month=excluded.once_month, once_day=excluded.once_day,
            repeat_days=excluded.repeat_days,
            enabled=excluded.enabled, label=excluded.label",
        params![
            device_id, id, req.hour, req.minute, repeat_kind, once_year, once_month, once_day,
            repeat_days, req.enabled, req.label
        ],
    )?;
    bump_version(&conn, device_id)?;
    Ok(id)
}

pub fn delete_alarm(db: &Db, device_id: &str, id: u8) -> Result<()> {
    let conn = db.lock().unwrap();
    conn.execute(
        "DELETE FROM alarms WHERE device_id = ?1 AND local_id = ?2",
        params![device_id, id],
    )?;
    bump_version(&conn, device_id)?;
    Ok(())
}

pub fn clear_alarms(db: &Db, device_id: &str) -> Result<()> {
    let conn = db.lock().unwrap();
    conn.execute(
        "DELETE FROM alarms WHERE device_id = ?1",
        params![device_id],
    )?;
    bump_version(&conn, device_id)
}

pub fn upsert_todo(
    db: &Db,
    device_id: &str,
    id: Option<u8>,
    req: &UpsertTodoRequest,
) -> Result<u8> {
    let conn = db.lock().unwrap();
    let id = match id {
        Some(id) => id,
        None => next_local_id(&conn, "todos", device_id)?,
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
    conn.execute(
        "INSERT INTO todos (device_id, local_id, text, done, importance, due_year, due_month, due_day, repeat_kind, repeat_days)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT(device_id, local_id) DO UPDATE SET
            text=excluded.text, done=excluded.done, importance=excluded.importance,
            due_year=excluded.due_year, due_month=excluded.due_month, due_day=excluded.due_day,
            repeat_kind=excluded.repeat_kind, repeat_days=excluded.repeat_days",
        params![
            device_id,
            id,
            req.text,
            req.done,
            req.importance.as_str(),
            due_year,
            due_month,
            due_day,
            repeat_kind,
            repeat_days
        ],
    )?;
    bump_version(&conn, device_id)?;
    Ok(id)
}

pub fn delete_todo(db: &Db, device_id: &str, id: u8) -> Result<()> {
    let conn = db.lock().unwrap();
    conn.execute(
        "DELETE FROM todos WHERE device_id = ?1 AND local_id = ?2",
        params![device_id, id],
    )?;
    bump_version(&conn, device_id)?;
    Ok(())
}

pub fn clear_todos(db: &Db, device_id: &str) -> Result<()> {
    let conn = db.lock().unwrap();
    conn.execute("DELETE FROM todos WHERE device_id = ?1", params![device_id])?;
    bump_version(&conn, device_id)
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// Parses the SQLite `importance` column; unknown/legacy values fall back
/// to `Medium`.
fn importance_from_str(s: &str) -> Importance {
    match s {
        "low" => Importance::Low,
        "high" => Importance::High,
        _ => Importance::Medium,
    }
}

/// Serializes a `Weekly`/`Monthly` day list to the JSON `repeat_days`
/// column (compact, spaces stripped).
fn repeat_days_json_opt(days: &[u8]) -> Option<String> {
    serde_json::to_string(days).ok().map(|s| s.replace(' ', ""))
}

/// Flattens a `Repeat` into the column tuple (`repeat_kind`,
/// `once_year`, `once_month`, `once_day`, `repeat_days`). `repeat_days`
/// holds the JSON array of covered days for `Weekly`/`Monthly` schedules.
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

/// Rebuilds a `Repeat` from the flattened columns; unknown/legacy values
/// fall back to `Daily`.
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

/// Parses the `repeat_days` JSON column into a `Vec<u8>`; empty/malformed
/// values become an empty vec (harmless - such a schedule simply never
/// fires and the UI should not produce it).
fn repeat_days_json(raw: Option<&str>) -> Vec<u8> {
    raw.and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bulk_clear_preserves_device_and_other_content() {
        let db = open(":memory:").unwrap();
        let device = register_device(&db, "test", None).unwrap();
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
        .unwrap();

        clear_alarms(&db, device.id.as_str()).unwrap();
        assert!(list_alarms(&db, device.id.as_str()).unwrap().is_empty());
        let todos = list_todos(&db, device.id.as_str()).unwrap();
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
        assert_eq!(list_devices(&db).unwrap().len(), 1);
    }

    #[test]
    fn device_merge_updates_flags_without_recreating_unknown_content() {
        let db = open(":memory:").unwrap();
        let device = register_device(&db, "test", None).unwrap();
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
        .unwrap();
        let before = find_device_by_token(&db, device.token.as_deref().unwrap())
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
            },
        )
        .unwrap();
        let todos = list_todos(&db, device.id.as_str()).unwrap();
        assert_eq!(todos.len(), 1);
        assert_eq!(todos[0].text, "server text");
        assert!(todos[0].done);
        assert_eq!(todos[0].importance, Importance::High);
        let after = find_device_by_token(&db, device.token.as_deref().unwrap())
            .unwrap()
            .unwrap()
            .1;
        assert_eq!(after, before + 1);
    }
}
