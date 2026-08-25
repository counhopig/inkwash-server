//! Sync business logic: applying what the physical device is allowed to
//! edit back onto server state. Split out of the old monolithic `db.rs`
//! alongside `schema.rs` (DDL/migrations) and `queries.rs` (CRUD).

use anyhow::Result;

use super::queries::bump_version;
use super::Db;
use crate::models::DeviceSyncRequest;

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
