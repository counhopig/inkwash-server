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

use crate::models::{Alarm, Device, Repeat, Todo, UpsertAlarmRequest, UpsertTodoRequest};

pub type Db = Arc<Mutex<Connection>>;

pub fn open(path: &str) -> Result<Db> {
    let conn = Connection::open(path).with_context(|| format!("failed to open {path}"))?;
    conn.pragma_update(None, "foreign_keys", true)?;
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS devices (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            token TEXT NOT NULL UNIQUE,
            version INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS alarms (
            device_id INTEGER NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
            local_id INTEGER NOT NULL,
            hour INTEGER NOT NULL,
            minute INTEGER NOT NULL,
            repeat_kind TEXT NOT NULL,
            once_year INTEGER,
            once_month INTEGER,
            once_day INTEGER,
            enabled INTEGER NOT NULL,
            label TEXT NOT NULL,
            PRIMARY KEY (device_id, local_id)
        );
        CREATE TABLE IF NOT EXISTS todos (
            device_id INTEGER NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
            local_id INTEGER NOT NULL,
            text TEXT NOT NULL,
            done INTEGER NOT NULL,
            PRIMARY KEY (device_id, local_id)
        );
        ",
    )?;
    Ok(Arc::new(Mutex::new(conn)))
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
pub fn register_device(db: &Db, name: &str) -> Result<Device> {
    let conn = db.lock().unwrap();
    let token = new_token();
    let now = now_unix();
    conn.execute(
        "INSERT INTO devices (name, token, version, created_at) VALUES (?1, ?2, 0, ?3)",
        params![name, token, now],
    )?;
    let id = conn.last_insert_rowid();
    Ok(Device {
        id,
        name: name.to_string(),
        token: Some(token),
    })
}

pub fn list_devices(db: &Db) -> Result<Vec<Device>> {
    let conn = db.lock().unwrap();
    let mut stmt = conn.prepare("SELECT id, name FROM devices ORDER BY id")?;
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

pub fn delete_device(db: &Db, device_id: i64) -> Result<()> {
    let conn = db.lock().unwrap();
    conn.execute("DELETE FROM devices WHERE id = ?1", params![device_id])?;
    Ok(())
}

/// Looks up the device owning `token`, returning `(device_id, version)`.
/// `version` becomes the ETag for `GET /api/sync` - see
/// `routes::device_sync`.
pub fn find_device_by_token(db: &Db, token: &str) -> Result<Option<(i64, i64)>> {
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

fn bump_version(conn: &Connection, device_id: i64) -> Result<()> {
    conn.execute(
        "UPDATE devices SET version = version + 1 WHERE id = ?1",
        params![device_id],
    )?;
    Ok(())
}

pub fn list_alarms(db: &Db, device_id: i64) -> Result<Vec<Alarm>> {
    let conn = db.lock().unwrap();
    let mut stmt = conn.prepare(
        "SELECT local_id, hour, minute, repeat_kind, once_year, once_month, once_day, enabled, label
         FROM alarms WHERE device_id = ?1 ORDER BY local_id",
    )?;
    let rows = stmt
        .query_map(params![device_id], |row| {
            let repeat_kind: String = row.get(3)?;
            let repeat = if repeat_kind == "once" {
                Repeat::Once {
                    year: row.get(4)?,
                    month: row.get(5)?,
                    day: row.get(6)?,
                }
            } else {
                Repeat::Daily
            };
            Ok(Alarm {
                id: row.get(0)?,
                hour: row.get(1)?,
                minute: row.get(2)?,
                repeat,
                enabled: row.get::<_, i64>(7)? != 0,
                label: row.get(8)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn list_todos(db: &Db, device_id: i64) -> Result<Vec<Todo>> {
    let conn = db.lock().unwrap();
    let mut stmt =
        conn.prepare("SELECT local_id, text, done FROM todos WHERE device_id = ?1 ORDER BY local_id")?;
    let rows = stmt
        .query_map(params![device_id], |row| {
            Ok(Todo {
                id: row.get(0)?,
                text: row.get(1)?,
                done: row.get::<_, i64>(2)? != 0,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn next_local_id(conn: &Connection, table: &str, device_id: i64) -> Result<u8> {
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
pub fn upsert_alarm(db: &Db, device_id: i64, id: Option<u8>, req: &UpsertAlarmRequest) -> Result<u8> {
    let conn = db.lock().unwrap();
    let id = match id {
        Some(id) => id,
        None => next_local_id(&conn, "alarms", device_id)?,
    };
    let (repeat_kind, once_year, once_month, once_day) = match req.repeat {
        Repeat::Daily => ("daily", None, None, None),
        Repeat::Once { year, month, day } => ("once", Some(year), Some(month), Some(day)),
    };
    conn.execute(
        "INSERT INTO alarms (device_id, local_id, hour, minute, repeat_kind, once_year, once_month, once_day, enabled, label)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT(device_id, local_id) DO UPDATE SET
            hour=excluded.hour, minute=excluded.minute, repeat_kind=excluded.repeat_kind,
            once_year=excluded.once_year, once_month=excluded.once_month, once_day=excluded.once_day,
            enabled=excluded.enabled, label=excluded.label",
        params![
            device_id, id, req.hour, req.minute, repeat_kind, once_year, once_month, once_day,
            req.enabled, req.label
        ],
    )?;
    bump_version(&conn, device_id)?;
    Ok(id)
}

pub fn delete_alarm(db: &Db, device_id: i64, id: u8) -> Result<()> {
    let conn = db.lock().unwrap();
    conn.execute(
        "DELETE FROM alarms WHERE device_id = ?1 AND local_id = ?2",
        params![device_id, id],
    )?;
    bump_version(&conn, device_id)?;
    Ok(())
}

pub fn upsert_todo(db: &Db, device_id: i64, id: Option<u8>, req: &UpsertTodoRequest) -> Result<u8> {
    let conn = db.lock().unwrap();
    let id = match id {
        Some(id) => id,
        None => next_local_id(&conn, "todos", device_id)?,
    };
    conn.execute(
        "INSERT INTO todos (device_id, local_id, text, done) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(device_id, local_id) DO UPDATE SET text=excluded.text, done=excluded.done",
        params![device_id, id, req.text, req.done],
    )?;
    bump_version(&conn, device_id)?;
    Ok(id)
}

pub fn delete_todo(db: &Db, device_id: i64, id: u8) -> Result<()> {
    let conn = db.lock().unwrap();
    conn.execute(
        "DELETE FROM todos WHERE device_id = ?1 AND local_id = ?2",
        params![device_id, id],
    )?;
    bump_version(&conn, device_id)?;
    Ok(())
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
