//! Wire types shared with the firmware's sync response (see
//! `inkpaper/docs/sync-api.md`). Field names/shapes here must match
//! `rust-firmware/src/alarms.rs`'s `StoredAlarm`/`Repeat` and
//! `rust-firmware/src/todos.rs`'s `Todo` exactly, since the firmware
//! deserializes this JSON directly into those types with no adapter layer.

use serde::{Deserialize, Serialize};

/// Externally-tagged to match the firmware's plain `serde` derive on
/// `enum Repeat { Daily, Weekly{..}, Monthly{..}, Once{..} }`: `Daily`
/// serializes as the bare string `"Daily"`, the rest as `{"Weekly": {...}}`
/// etc. Weekdays are 0=Sunday..6=Saturday; month days are 1..=31.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Repeat {
    Daily,
    Weekly { days: Vec<u8> },
    Monthly { days: Vec<u8> },
    Once { year: u16, month: u8, day: u8 },
}

impl Repeat {
    /// Stable discriminator used as the SQLite `repeat_kind` column.
    pub const fn kind(&self) -> &'static str {
        match self {
            Repeat::Daily => "daily",
            Repeat::Weekly { .. } => "weekly",
            Repeat::Monthly { .. } => "monthly",
            Repeat::Once { .. } => "once",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Alarm {
    pub id: u8,
    pub hour: u8,
    pub minute: u8,
    pub repeat: Repeat,
    pub enabled: bool,
    pub label: String,
}

/// Todo importance, serialized snake_case (`"low"`/`"medium"`/`"high"`) to
/// match the firmware's `todos::Importance`. `Medium` is the default so
/// records created before importance existed stay comparable.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Importance {
    Low,
    #[default]
    Medium,
    High,
}

impl Importance {
    /// Stable string used as the SQLite column value (`low`/`medium`/`high`).
    pub const fn as_str(self) -> &'static str {
        match self {
            Importance::Low => "low",
            Importance::Medium => "medium",
            Importance::High => "high",
        }
    }
}

/// Full due date of a todo. The `year` field defaults to 0 for records
/// synced before it existed; such todos have no concrete date and simply
/// don't mark the device calendar or remind until re-edited.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TodoDue {
    #[serde(default)]
    pub year: u16,
    pub month: u8,
    pub day: u8,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Todo {
    pub id: u8,
    pub text: String,
    pub done: bool,
    #[serde(default)]
    pub importance: Importance,
    /// Single due date (used when `repeat` is `None`).
    #[serde(default)]
    pub due_date: Option<TodoDue>,
    /// Recurrence schedule; when set, the todo is due on every date the
    /// schedule covers instead of just `due_date`.
    #[serde(default)]
    pub repeat: Option<Repeat>,
}

/// Inbox notification kind, serialized snake_case to match the firmware's
/// `InboxKind` (`"alert"`/`"event"`/`"info"`).
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InboxKind {
    Alert,
    Event,
    Info,
}

impl From<&str> for InboxKind {
    fn from(s: &str) -> Self {
        match s {
            "alert" => InboxKind::Alert,
            "event" => InboxKind::Event,
            _ => InboxKind::Info,
        }
    }
}

/// Inbox notification priority, serialized snake_case to match the
/// firmware's `Priority`. `High` drives on-device urgent behavior: the
/// firmware shows a full-screen reminder with an insistent tone as soon as
/// the message arrives on the next sync. `Normal` is the default.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    #[default]
    Normal,
    High,
}

impl From<&str> for Priority {
    fn from(s: &str) -> Self {
        match s {
            "high" => Priority::High,
            _ => Priority::Normal,
        }
    }
}

/// A single inbox notification as seen by the device over the sync wire.
/// `id` is the device-visible stable `seq` (u64); the server's internal
/// UUID id is never sent to the device.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InboxItem {
    pub id: u64,
    pub kind: InboxKind,
    #[serde(default)]
    pub priority: Priority,
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub when: Option<i64>,
    #[serde(default)]
    pub read: bool,
}

/// Body of a successful (HTTP 200) `GET /api/sync` response - see
/// `inkpaper/docs/sync-api.md`.
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
