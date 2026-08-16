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
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};

use crate::db::{self, Db};
use crate::models::{RegisterDeviceRequest, SyncResponse, UpsertAlarmRequest, UpsertTodoRequest};

#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub admin_token: String,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/sync", get(device_sync))
        .route("/api/devices", post(register_device).get(list_devices))
        .route("/api/devices/:device_id", delete(delete_device))
        .route(
            "/api/devices/:device_id/alarms",
            get(list_alarms).post(create_alarm),
        )
        .route(
            "/api/devices/:device_id/alarms/:alarm_id",
            put(update_alarm).delete(delete_alarm),
        )
        .route(
            "/api/devices/:device_id/todos",
            get(list_todos).post(create_todo),
        )
        .route(
            "/api/devices/:device_id/todos/:todo_id",
            put(update_todo).delete(delete_todo),
        )
        .with_state(state)
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
}

fn require_admin(headers: &HeaderMap, state: &AppState) -> Result<(), Response> {
    match bearer_token(headers) {
        Some(token) if token == state.admin_token => Ok(()),
        _ => Err((StatusCode::UNAUTHORIZED, "missing or invalid admin token").into_response()),
    }
}

fn internal_error(err: anyhow::Error) -> Response {
    tracing::error!("{err:#}");
    (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response()
}

// --- Device-facing sync endpoint (docs/sync-api.md) -------------------

async fn device_sync(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Some(token) = bearer_token(&headers) else {
        return (StatusCode::UNAUTHORIZED, "missing bearer token").into_response();
    };
    let (device_id, version) = match db::find_device_by_token(&state.db, token) {
        Ok(Some(v)) => v,
        Ok(None) => return (StatusCode::UNAUTHORIZED, "unknown device token").into_response(),
        Err(err) => return internal_error(err),
    };

    let etag = format!("\"v{version}\"");
    let if_none_match = headers
        .get(axum::http::header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok());
    if if_none_match == Some(etag.as_str()) {
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
    (
        StatusCode::OK,
        [(axum::http::header::ETAG, etag)],
        body,
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
        return resp;
    }
    match db::register_device(&state.db, &req.name) {
        Ok(device) => (StatusCode::CREATED, Json(device)).into_response(),
        Err(err) => internal_error(err),
    }
}

async fn list_devices(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(resp) = require_admin(&headers, &state) {
        return resp;
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
        return resp;
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
        return resp;
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
        return resp;
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
        return resp;
    }
    match db::upsert_alarm(&state.db, device_id, Some(alarm_id), &req) {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(err) => internal_error(err),
    }
}

async fn delete_alarm(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((device_id, alarm_id)): Path<(i64, u8)>,
) -> Response {
    if let Err(resp) = require_admin(&headers, &state) {
        return resp;
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
        return resp;
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
        return resp;
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
        return resp;
    }
    match db::upsert_todo(&state.db, device_id, Some(todo_id), &req) {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(err) => internal_error(err),
    }
}

async fn delete_todo(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((device_id, todo_id)): Path<(i64, u8)>,
) -> Response {
    if let Err(resp) = require_admin(&headers, &state) {
        return resp;
    }
    match db::delete_todo(&state.db, device_id, todo_id) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(err) => internal_error(err),
    }
}
