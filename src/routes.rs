//! HTTP handlers. Two trust domains share this router:
//! - `/api/*` (except `/api/sync`) is the admin/management surface used by
//!   `inkpaper-desktop` to register devices and edit their alarms/todos.
//!   Guarded by a single fixed `ADMIN_TOKEN` (see `main.rs`) - this is a
//!   personal server for one owner, not a multi-tenant service, so a
//!   single shared admin credential is enough.
//! - `/api/sync` is the device-facing endpoint from
//!   `inkpaper/docs/sync-api.md`, guarded per-device by the token
//!   `register_device` issued.

use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use rust_embed::RustEmbed;

use crate::db::{self, Db};
use crate::models::{
    DeviceSyncRequest, RegisterDeviceRequest, SyncResponse, UpsertAlarmRequest, UpsertTodoRequest,
};

/// The built admin console (`admin-ui/`, a small Vue 3 + Vite app - see
/// that directory's README) embedded into the binary at compile time, so
/// the server stays a single deployable artifact with no extra static
/// files to ship alongside it. Run `npm run build` in `admin-ui/` before
/// `cargo build`/`cargo run` to regenerate `admin-ui/dist/`.
#[derive(RustEmbed)]
#[folder = "admin-ui/dist/"]
struct AdminAssets;

#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub admin_token: String,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(admin_index))
        .route("/assets/*path", get(admin_asset))
        .route("/health", get(health))
        .route("/api/sync", get(device_sync).post(device_push_sync))
        .route("/api/devices", post(register_device).get(list_devices))
        .route("/api/devices/:device_id", delete(delete_device))
        .route(
            "/api/devices/:device_id/alarms",
            get(list_alarms).post(create_alarm).delete(clear_alarms),
        )
        .route(
            "/api/devices/:device_id/alarms/:alarm_id",
            put(update_alarm).delete(delete_alarm),
        )
        .route(
            "/api/devices/:device_id/todos",
            get(list_todos).post(create_todo).delete(clear_todos),
        )
        .route(
            "/api/devices/:device_id/todos/:todo_id",
            put(update_todo).delete(delete_todo),
        )
        .with_state(state)
}

async fn admin_index() -> Response {
    serve_admin_asset("index.html")
}

async fn admin_asset(Path(path): Path<String>) -> Response {
    // Embedded keys are relative to admin-ui/dist/ (e.g. "assets/index-*.js"),
    // but the wildcard route only captures what follows "/assets/".
    serve_admin_asset(&format!("assets/{path}"))
}

fn serve_admin_asset(path: &str) -> Response {
    match AdminAssets::get(path) {
        Some(file) => {
            let mime = file.metadata.mimetype();
            ([(header::CONTENT_TYPE, mime)], file.data).into_response()
        }
        None => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

async fn health() -> impl IntoResponse {
    Json(serde_json::json!({ "status": "ok" }))
}

fn bad_request(message: impl Into<String>) -> Response {
    (StatusCode::BAD_REQUEST, message.into()).into_response()
}

fn validate_alarm(req: &UpsertAlarmRequest) -> Result<(), &'static str> {
    if req.hour > 23 || req.minute > 59 {
        return Err("hour must be 0..23 and minute must be 0..59");
    }
    if req.label.chars().count() > 40 {
        return Err("alarm label must be at most 40 characters");
    }
    if let crate::models::Repeat::Once { year, month, day } = req.repeat {
        if !(1..=12).contains(&month) || !(1..=31).contains(&day) || year < 1970 {
            return Err("invalid once-alarm date");
        }
    }
    Ok(())
}

fn validate_todo(req: &UpsertTodoRequest) -> Result<(), &'static str> {
    if req.text.trim().is_empty() {
        return Err("todo text must not be empty");
    }
    if req.text.chars().count() > 120 {
        return Err("todo text must be at most 120 characters");
    }
    Ok(())
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
}

fn require_admin(headers: &HeaderMap, state: &AppState) -> Result<(), (StatusCode, &'static str)> {
    match bearer_token(headers) {
        Some(token) if token == state.admin_token => Ok(()),
        _ => Err((StatusCode::UNAUTHORIZED, "missing or invalid admin token")),
    }
}

fn internal_error(err: anyhow::Error) -> Response {
    tracing::error!("{err:#}");
    (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response()
}

// --- Device-facing sync endpoint (docs/sync-api.md) -------------------

async fn device_sync(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Some(token) = bearer_token(&headers) else {
        tracing::warn!("sync rejected: missing bearer token");
        return (StatusCode::UNAUTHORIZED, "missing bearer token").into_response();
    };
    let (device_id, version) = match db::find_device_by_token(&state.db, token) {
        Ok(Some(v)) => v,
        Ok(None) => {
            tracing::warn!("sync rejected: unknown device token");
            return (StatusCode::UNAUTHORIZED, "unknown device token").into_response();
        }
        Err(err) => return internal_error(err),
    };

    // Include device identity so a cached ETag from an old/re-registered
    // device can never suppress the first payload for a different device.
    let etag = format!("\"d{device_id}-v{version}\"");
    let if_none_match = headers
        .get(axum::http::header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok());
    if if_none_match == Some(etag.as_str()) {
        tracing::info!(device_id, version, "sync not modified");
        return StatusCode::NOT_MODIFIED.into_response();
    }

    let alarms = match db::list_alarms(&state.db, device_id) {
        Ok(v) => v,
        Err(err) => return internal_error(err),
    };
    let todos = match db::list_todos(&state.db, device_id) {
        Ok(v) => v,
        Err(err) => return internal_error(err),
    };

    let body = Json(SyncResponse { alarms, todos });
    tracing::info!(
        device_id,
        version,
        alarm_count = body.0.alarms.len(),
        todo_count = body.0.todos.len(),
        "sync payload served"
    );
    (StatusCode::OK, [(axum::http::header::ETAG, etag)], body).into_response()
}

async fn device_push_sync(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<DeviceSyncRequest>,
) -> Response {
    let Some(token) = bearer_token(&headers) else {
        return (StatusCode::UNAUTHORIZED, "missing bearer token").into_response();
    };
    let device_id = match db::find_device_by_token(&state.db, token) {
        Ok(Some((id, _))) => id,
        Ok(None) => return (StatusCode::UNAUTHORIZED, "unknown device token").into_response(),
        Err(err) => return internal_error(err),
    };
    let version = match db::merge_device_state(&state.db, device_id, &req) {
        Ok(version) => version,
        Err(err) => return internal_error(err),
    };
    let alarms = match db::list_alarms(&state.db, device_id) {
        Ok(value) => value,
        Err(err) => return internal_error(err),
    };
    let todos = match db::list_todos(&state.db, device_id) {
        Ok(value) => value,
        Err(err) => return internal_error(err),
    };
    let etag = format!("\"d{device_id}-v{version}\"");
    tracing::info!(
        device_id,
        version,
        alarm_count = alarms.len(),
        todo_count = todos.len(),
        "device state merged and sync payload served"
    );
    (
        StatusCode::OK,
        [(axum::http::header::ETAG, etag)],
        Json(SyncResponse { alarms, todos }),
    )
        .into_response()
}

// --- Admin: devices -----------------------------------------------------

async fn register_device(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<RegisterDeviceRequest>,
) -> Response {
    if let Err(resp) = require_admin(&headers, &state) {
        return resp.into_response();
    }
    if req.name.trim().is_empty() || req.name.chars().count() > 80 {
        return bad_request("device name must be 1..80 characters");
    }
    match db::register_device(&state.db, &req.name) {
        Ok(device) => (StatusCode::CREATED, Json(device)).into_response(),
        Err(err) => internal_error(err),
    }
}

async fn list_devices(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(resp) = require_admin(&headers, &state) {
        return resp.into_response();
    }
    match db::list_devices(&state.db) {
        Ok(devices) => Json(devices).into_response(),
        Err(err) => internal_error(err),
    }
}

async fn delete_device(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(device_id): Path<i64>,
) -> Response {
    if let Err(resp) = require_admin(&headers, &state) {
        return resp.into_response();
    }
    match db::delete_device(&state.db, device_id) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(err) => internal_error(err),
    }
}

// --- Admin: alarms --------------------------------------------------------

async fn list_alarms(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(device_id): Path<i64>,
) -> Response {
    if let Err(resp) = require_admin(&headers, &state) {
        return resp.into_response();
    }
    match db::list_alarms(&state.db, device_id) {
        Ok(alarms) => Json(alarms).into_response(),
        Err(err) => internal_error(err),
    }
}

async fn create_alarm(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(device_id): Path<i64>,
    Json(req): Json<UpsertAlarmRequest>,
) -> Response {
    if let Err(resp) = require_admin(&headers, &state) {
        return resp.into_response();
    }
    if let Err(msg) = validate_alarm(&req) {
        return bad_request(msg);
    }
    match db::upsert_alarm(&state.db, device_id, None, &req) {
        Ok(id) => (StatusCode::CREATED, Json(serde_json::json!({ "id": id }))).into_response(),
        Err(err) => internal_error(err),
    }
}

async fn update_alarm(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((device_id, alarm_id)): Path<(i64, u8)>,
    Json(req): Json<UpsertAlarmRequest>,
) -> Response {
    if let Err(resp) = require_admin(&headers, &state) {
        return resp.into_response();
    }
    if let Err(msg) = validate_alarm(&req) {
        return bad_request(msg);
    }
    match db::upsert_alarm(&state.db, device_id, Some(alarm_id), &req) {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(err) => internal_error(err),
    }
}

async fn clear_alarms(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(device_id): Path<i64>,
) -> Response {
    if let Err(resp) = require_admin(&headers, &state) {
        return resp.into_response();
    }
    match db::clear_alarms(&state.db, device_id) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(err) => internal_error(err),
    }
}

async fn delete_alarm(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((device_id, alarm_id)): Path<(i64, u8)>,
) -> Response {
    if let Err(resp) = require_admin(&headers, &state) {
        return resp.into_response();
    }
    match db::delete_alarm(&state.db, device_id, alarm_id) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(err) => internal_error(err),
    }
}

// --- Admin: todos ---------------------------------------------------------

async fn list_todos(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(device_id): Path<i64>,
) -> Response {
    if let Err(resp) = require_admin(&headers, &state) {
        return resp.into_response();
    }
    match db::list_todos(&state.db, device_id) {
        Ok(todos) => Json(todos).into_response(),
        Err(err) => internal_error(err),
    }
}

async fn create_todo(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(device_id): Path<i64>,
    Json(req): Json<UpsertTodoRequest>,
) -> Response {
    if let Err(resp) = require_admin(&headers, &state) {
        return resp.into_response();
    }
    if let Err(msg) = validate_todo(&req) {
        return bad_request(msg);
    }
    match db::upsert_todo(&state.db, device_id, None, &req) {
        Ok(id) => (StatusCode::CREATED, Json(serde_json::json!({ "id": id }))).into_response(),
        Err(err) => internal_error(err),
    }
}

async fn update_todo(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((device_id, todo_id)): Path<(i64, u8)>,
    Json(req): Json<UpsertTodoRequest>,
) -> Response {
    if let Err(resp) = require_admin(&headers, &state) {
        return resp.into_response();
    }
    if let Err(msg) = validate_todo(&req) {
        return bad_request(msg);
    }
    match db::upsert_todo(&state.db, device_id, Some(todo_id), &req) {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(err) => internal_error(err),
    }
}

async fn clear_todos(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(device_id): Path<i64>,
) -> Response {
    if let Err(resp) = require_admin(&headers, &state) {
        return resp.into_response();
    }
    match db::clear_todos(&state.db, device_id) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(err) => internal_error(err),
    }
}

async fn delete_todo(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((device_id, todo_id)): Path<(i64, u8)>,
) -> Response {
    if let Err(resp) = require_admin(&headers, &state) {
        return resp.into_response();
    }
    match db::delete_todo(&state.db, device_id, todo_id) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(err) => internal_error(err),
    }
}
