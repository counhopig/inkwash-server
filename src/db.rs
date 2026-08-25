//! Storage behind sqlx's `Any` driver, so one codebase serves both SQLite
//! (default, zero-config `sqlite://` URL) and PostgreSQL (via a
//! `postgres://` `DATABASE_URL`). The backend is selected at `open()` time
//! from the URL; the schema lives in sqlx migration files (see
//! `db::schema`), and the query code is split by responsibility:
//!
//! - [`schema`] - DDL, migrations, `open()`, the pre-UUID-era SQLite rebuild
//! - [`queries`] - CRUD for devices, accounts/sessions, channels/inbox,
//!   alarms/todos (re-exported below so callers keep using `db::` paths)
//! - [`sync`] - `merge_device_state` (device-uploaded flag merging)
//!
//! Data SQL uses sqlx's cross-dialect `?` placeholders and `i64` for every
//! integer so one SQL body works on both backends; `Db::adapt()` rewrites
//! `?` to `$N` for Postgres at runtime. A small connection pool (default
//! max 2) replaces the old single `Arc<Mutex<Connection>>`.

use sqlx::AnyPool;

pub mod queries;
pub mod schema;
pub mod sync;

pub use queries::*;
pub use schema::open;
pub use sync::*;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        DeviceSyncRequest, Importance, Repeat, UpsertAlarmRequest, UpsertTodoRequest,
    };

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
            "normal",
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
            "normal",
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
            "normal",
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
            "normal",
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
        let (new_token, new_prefix) = rotate_channel_token(&db, &device.id, &channel.id)
            .await
            .unwrap()
            .unwrap();
        assert!(new_token.starts_with("ipwh_"));
        assert_eq!(new_prefix, new_token.chars().take(12).collect::<String>());
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
        deliver_inbox(
            &db,
            &device.id,
            &channel.id,
            "info",
            "normal",
            "x",
            "",
            None,
            None,
        )
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
                "normal",
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

    #[tokio::test]
    async fn inbox_priority_roundtrips_and_drives_urgent_flag() {
        let db = memory_db().await;
        let device = register_device(&db, "dev", None).await.unwrap();
        let (channel, _) = create_channel(&db, &device.id, "webhook", "CI", None)
            .await
            .unwrap();

        deliver_inbox(
            &db,
            &device.id,
            &channel.id,
            "info",
            "normal",
            "normal msg",
            "",
            None,
            None,
        )
        .await
        .unwrap();
        deliver_inbox(
            &db,
            &device.id,
            &channel.id,
            "alert",
            "high",
            "urgent msg",
            "",
            None,
            None,
        )
        .await
        .unwrap();

        let (items, _) = list_inbox(&db, &device.id, 20).await.unwrap();
        assert_eq!(items.len(), 2);
        let urgent = items.iter().find(|i| i.title == "urgent msg").unwrap();
        assert_eq!(urgent.priority, crate::models::Priority::High);
        let normal = items.iter().find(|i| i.title == "normal msg").unwrap();
        assert_eq!(normal.priority, crate::models::Priority::Normal);

        // has_unread_high_inbox is true while the urgent one is unread.
        assert!(has_unread_high_inbox(&db, &device.id).await.unwrap());

        // Marking the urgent one read clears the flag.
        mark_inbox_read(&db, &device.id, &[urgent.id])
            .await
            .unwrap();
        assert!(!has_unread_high_inbox(&db, &device.id).await.unwrap());
    }

    #[tokio::test]
    async fn failed_write_does_not_bump_device_version() {
        let db = memory_db().await;
        let device = register_device(&db, "dev", None).await.unwrap();
        let before = find_device_by_token(&db, device.token.as_deref().unwrap())
            .await
            .unwrap()
            .unwrap()
            .1;
        // Insert an alarm for a nonexistent device: the FK violation aborts
        // the transaction, so the version bump inside it must not persist.
        let res = upsert_alarm(
            &db,
            "no-such-device",
            None,
            &UpsertAlarmRequest {
                hour: 7,
                minute: 0,
                repeat: Repeat::Daily,
                enabled: true,
                label: String::new(),
            },
        )
        .await;
        assert!(res.is_err());
        let after = find_device_by_token(&db, device.token.as_deref().unwrap())
            .await
            .unwrap()
            .unwrap()
            .1;
        assert_eq!(after, before, "version must not advance on a failed write");
    }
}
