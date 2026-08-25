//! Wire types shared with the firmware's sync response (see
//! `inkwash/docs/sync-api.md`).
//!
//! `Repeat`/`Alarm`/`Importance`/`TodoDue`/`Todo`/`InboxKind`/`Priority`/
//! `InboxItem` are re-exports of `inkwash-logic`'s definitions (see the
//! `inkwash-logic` dependency in `Cargo.toml`) rather than a hand-copied
//! shape that could drift from the firmware's own - the firmware
//! deserializes this JSON directly into those same Rust types with no
//! adapter layer, so this crate and `rust-firmware` now literally share
//! one definition instead of two independently-maintained ones that
//! happened to agree. `Alarm` is `StoredAlarm` under its server-side name
//! so every existing `models::Alarm` call site keeps working unchanged.
//!
//! Everything below that line is server-only - HTTP request/response DTOs
//! whose JSON shape happens to match what the firmware expects, but that
//! are never the literal same Rust value crossing a process boundary the
//! way the re-exports above are, so keeping them as this crate's own types
//! costs nothing and preserves the `skip_serializing_if` compactness the
//! shared `SyncResponse` in `inkwash-logic` doesn't need to care about.

use serde::{Deserialize, Serialize};

pub use inkwash_logic::alarm_schedule::{Repeat, StoredAlarm as Alarm};
pub use inkwash_logic::inbox_item::{InboxItem, InboxKind, Priority};
pub use inkwash_logic::todo::{Importance, Todo, TodoDue};

/// Body of a successful (HTTP 200) `GET /api/sync` response - see
/// `inkwash/docs/sync-api.md`.
#[derive(Clone, Debug, Serialize)]
pub struct SyncResponse {
    pub alarms: Vec<Alarm>,
    pub todos: Vec<Todo>,
    /// Inbox notifications delivered to the device (capacity-capped).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inbox: Vec<InboxItem>,
    /// The device reported these `seq`s as read; echoed back so the
    /// firmware can drop them from its pending-read set.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inbox_read_acked: Vec<u64>,
    /// True when the server had more inbox items than it could fit in the
    /// response; the device shows a "more on server" hint.
    #[serde(default)]
    pub inbox_truncated: bool,
}

/// Mutable device-side state uploaded before the server returns its
/// authoritative content. Text, schedules and membership remain managed by
/// Desktop/Server; the device may only report completion/enabled flags for
/// records that still exist on the server.
#[derive(Clone, Debug, Deserialize)]
pub struct DeviceSyncRequest {
    #[serde(default)]
    pub alarms: Vec<DeviceAlarmState>,
    #[serde(default)]
    pub todos: Vec<DeviceTodoState>,
    /// Device-local inbox `seq`s the user has read; unknown ids are ignored.
    #[serde(default)]
    pub inbox_read: Vec<u64>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct DeviceAlarmState {
    pub id: u8,
    pub enabled: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct DeviceTodoState {
    pub id: u8,
    pub done: bool,
    /// `None` when an older firmware uploads without importance - only
    /// present states are applied.
    #[serde(default)]
    pub importance: Option<Importance>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Device {
    /// UUID (v4) - opaque string, not an enumerable integer, so ids can't
    /// be guessed or iterated by an attacker who steals one url.
    pub id: String,
    pub name: String,
    /// Only ever returned once, at registration time - not readable again
    /// afterward (see `db::register_device`'s doc comment).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct RegisterDeviceRequest {
    pub name: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct UpsertAlarmRequest {
    pub hour: u8,
    pub minute: u8,
    pub repeat: Repeat,
    pub enabled: bool,
    #[serde(default)]
    pub label: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct UpsertTodoRequest {
    pub text: String,
    #[serde(default)]
    pub done: bool,
    #[serde(default)]
    pub importance: Importance,
    #[serde(default)]
    pub due_date: Option<TodoDue>,
    #[serde(default)]
    pub repeat: Option<Repeat>,
}

// --- Console accounts -----------------------------------------------------
// These types are admin-console-only; they don't cross the sync wire
// contract (the firmware never sees them).

#[derive(Clone, Debug, Serialize)]
pub struct Account {
    pub id: i64,
    pub username: String,
    pub created_at: i64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct AuthRequest {
    pub username: String,
    pub password: String,
}

/// Session credential returned by register/login. The client sends it back
/// as a bearer token, exactly like the `ADMIN_TOKEN`.
#[derive(Clone, Debug, Serialize)]
pub struct AuthResponse {
    pub token: String,
    pub username: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ChangePasswordRequest {
    pub old_password: String,
    pub new_password: String,
}

/// Admin-only view of an account (never exposes the password hash).
#[derive(Clone, Debug, Serialize)]
pub struct AccountSummary {
    pub id: i64,
    pub username: String,
    pub created_at: i64,
    pub device_count: i64,
    pub session_count: i64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct AdminResetPasswordRequest {
    pub new_password: String,
}

// --- External channels & inbox ---------------------------------------------

/// A configured external source (webhook / CalDAV) bound to one device.
/// This is the admin-facing view; it never contains the plaintext token or
/// decrypted CalDAV credentials.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Channel {
    pub id: String,
    pub device_id: String,
    /// `"webhook"` or `"caldav_basic"` (Phase 1 implements webhook).
    pub kind: String,
    pub name: String,
    pub enabled: bool,
    /// Short prefix of the webhook token for human identification (never a
    /// credential). Empty for non-webhook channels.
    pub token_prefix: String,
    pub last_sync_at: Option<i64>,
    pub last_sync_error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct CreateChannelRequest {
    pub kind: String,
    pub name: String,
    /// CalDAV-only: `{ url, username, password }` (encrypted server-side).
    /// Reserved for Phase 2; webhook channels ignore it.
    #[serde(default)]
    #[allow(dead_code)]
    pub config: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct UpdateChannelRequest {
    pub name: Option<String>,
    pub enabled: Option<bool>,
}

/// Response to `POST .../channels` for a webhook channel: the plaintext
/// token is returned exactly once here.
#[derive(Clone, Debug, Serialize)]
pub struct ChannelCreated {
    pub channel: Channel,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivery_url: Option<String>,
}

/// Webhook delivery payload (`POST /api/channels/:id/messages`).
#[derive(Clone, Debug, Deserialize)]
pub struct InboxCreateRequest {
    pub kind: String,
    #[serde(default)]
    pub priority: Option<Priority>,
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub when: Option<i64>,
}

/// Response to a successful webhook delivery.
#[derive(Clone, Debug, Serialize)]
pub struct InboxAccepted {
    pub accepted: bool,
    pub id: u64,
}
