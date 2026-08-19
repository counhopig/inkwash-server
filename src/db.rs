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

use crate::models::{
    Alarm, Device, DeviceSyncRequest, Importance, Repeat, Todo, UpsertAlarmRequest,
    UpsertTodoRequest,
};

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
    // Migrate databases created before importance/due_date existed:
    // `CREATE TABLE IF NOT EXISTS` never adds columns, so add them
    // explicitly and ignore the duplicate-column error on fresh DBs.
    for stmt in [
        "ALTER TABLE todos ADD COLUMN importance TEXT NOT NULL DEFAULT 'medium'",
        "ALTER TABLE todos ADD COLUMN due_month INTEGER",
        "ALTER TABLE todos ADD COLUMN due_day INTEGER",
    ] {
        let _ = conn.execute(stmt, []);
    }
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

/// Applies only the fields the physical device is allowed to edit. Unknown
/// IDs are ignored so stale device caches cannot recreate content deleted by
/// Desktop/Server. Returns the resulting device version.
pub fn merge_device_state(db: &Db, device_id: i64, state: &DeviceSyncRequest) -> Result<i64> {
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
    let mut stmt = conn.prepare(
        "SELECT local_id, text, done, importance, due_month, due_day
         FROM todos WHERE device_id = ?1 ORDER BY local_id",
    )?;
    let rows = stmt
        .query_map(params![device_id], |row| {
            let importance_str: String = row.get(3)?;
            let due_month: Option<i64> = row.get(4)?;
            let due_day: Option<i64> = row.get(5)?;
            Ok(Todo {
                id: row.get(0)?,
                text: row.get(1)?,
                done: row.get::<_, i64>(2)? != 0,
                importance: importance_from_str(&importance_str),
                due_date: match (due_month, due_day) {
                    (Some(month), Some(day)) => Some(crate::models::TodoDue {
                        month: month as u8,
                        day: day as u8,
                    }),
                    _ => None,
                },
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
pub fn upsert_alarm(
    db: &Db,
    device_id: i64,
    id: Option<u8>,
    req: &UpsertAlarmRequest,
) -> Result<u8> {
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

pub fn clear_alarms(db: &Db, device_id: i64) -> Result<()> {
    let conn = db.lock().unwrap();
    conn.execute(
        "DELETE FROM alarms WHERE device_id = ?1",
        params![device_id],
    )?;
    bump_version(&conn, device_id)
}

pub fn upsert_todo(db: &Db, device_id: i64, id: Option<u8>, req: &UpsertTodoRequest) -> Result<u8> {
    let conn = db.lock().unwrap();
    let id = match id {
        Some(id) => id,
        None => next_local_id(&conn, "todos", device_id)?,
    };
    let (due_month, due_day) = match req.due_date {
        Some(due) => (Some(due.month as i64), Some(due.day as i64)),
        None => (None, None),
    };
    conn.execute(
        "INSERT INTO todos (device_id, local_id, text, done, importance, due_month, due_day)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(device_id, local_id) DO UPDATE SET
            text=excluded.text, done=excluded.done, importance=excluded.importance,
            due_month=excluded.due_month, due_day=excluded.due_day",
        params![
            device_id,
            id,
            req.text,
            req.done,
            req.importance.as_str(),
            due_month,
            due_day
        ],
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

pub fn clear_todos(db: &Db, device_id: i64) -> Result<()> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bulk_clear_preserves_device_and_other_content() {
        let db = open(":memory:").unwrap();
        let device = register_device(&db, "test").unwrap();
        upsert_alarm(
            &db,
            device.id,
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
            device.id,
            None,
            &UpsertTodoRequest {
                text: "keep".to_string(),
                done: false,
                importance: Importance::High,
                due_date: Some(crate::models::TodoDue { month: 12, day: 25 }),
            },
        )
        .unwrap();

        clear_alarms(&db, device.id).unwrap();
        assert!(list_alarms(&db, device.id).unwrap().is_empty());
        let todos = list_todos(&db, device.id).unwrap();
        assert_eq!(todos.len(), 1);
        assert_eq!(todos[0].importance, Importance::High);
        assert_eq!(
            todos[0].due_date,
            Some(crate::models::TodoDue { month: 12, day: 25 })
        );
        assert_eq!(list_devices(&db).unwrap().len(), 1);
    }

    #[test]
    fn device_merge_updates_flags_without_recreating_unknown_content() {
        let db = open(":memory:").unwrap();
        let device = register_device(&db, "test").unwrap();
        let todo_id = upsert_todo(
            &db,
            device.id,
            None,
            &UpsertTodoRequest {
                text: "server text".to_string(),
                done: false,
                importance: Importance::Low,
                due_date: None,
            },
        )
        .unwrap();
        let before = find_device_by_token(&db, device.token.as_deref().unwrap())
            .unwrap()
            .unwrap()
            .1;
        merge_device_state(
            &db,
            device.id,
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
        let todos = list_todos(&db, device.id).unwrap();
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
