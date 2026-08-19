//! Wire types shared with the firmware's sync response (see
//! `inkpaper/docs/sync-api.md`). Field names/shapes here must match
//! `rust-firmware/src/alarms.rs`'s `StoredAlarm`/`Repeat` and
//! `rust-firmware/src/todos.rs`'s `Todo` exactly, since the firmware
//! deserializes this JSON directly into those types with no adapter layer.

use serde::{Deserialize, Serialize};

/// Externally-tagged to match the firmware's plain `serde` derive on
/// `enum Repeat { Daily, Once { year, month, day } }`: `Daily` serializes
/// as the bare string `"Daily"`, `Once` as `{"Once": {"year":...}}`.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Repeat {
    Daily,
    Once { year: u16, month: u8, day: u8 },
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

/// Optional due date of a todo - month/day without a year (a recurring
/// "every year on this date" semantics is deliberately out of scope; the
/// device calendar shows the marker on that day of the current month).
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TodoDue {
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
    #[serde(default)]
    pub due_date: Option<TodoDue>,
}

/// Body of a successful (HTTP 200) `GET /api/sync` response - see
/// `inkpaper/docs/sync-api.md`.
#[derive(Clone, Debug, Serialize)]
pub struct SyncResponse {
    pub alarms: Vec<Alarm>,
    pub todos: Vec<Todo>,
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
}
