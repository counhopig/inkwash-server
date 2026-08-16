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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Todo {
    pub id: u8,
    pub text: String,
    pub done: bool,
}

/// Body of a successful (HTTP 200) `GET /api/sync` response - see
/// `inkpaper/docs/sync-api.md`.
#[derive(Clone, Debug, Serialize)]
pub struct SyncResponse {
    pub alarms: Vec<Alarm>,
    pub todos: Vec<Todo>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Device {
    pub id: i64,
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
}
