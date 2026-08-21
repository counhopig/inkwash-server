//! HTTP handlers. Three trust domains share this router:
//! - `/api/*` (except `/api/sync`) is the admin/management surface used by
//!   `inkpaper-desktop` to register devices and edit their alarms/todos.
//!   Guarded by either the single fixed `ADMIN_TOKEN` (see `main.rs`) - a
//!   personal server for one owner - or a console-account session.
//! - `/api/auth/*` is the console account surface (register/login/logout/
//!   change password). Account sessions are bearer tokens stored in the DB.
//! - `/api/sync` is the device-facing endpoint from
//!   `inkpaper/docs/sync-api.md`, guarded per-device by the token
//!   `register_device` issued.
//!
//! Scope rules: a console account can only see/register/manage its own
//! devices (`devices.account_id`); the `ADMIN_TOKEN` can see and manage
//! everything, including unowned devices (the ones the desktop tool
//! registers), so existing desktop workflows keep working unchanged.

use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use rust_embed::RustEmbed;

use crate::db::{self, Db};
use crate::models::{
    Account, AdminResetPasswordRequest, AuthRequest, AuthResponse, ChangePasswordRequest,
    ChannelCreated, CreateChannelRequest, DeviceSyncRequest, InboxAccepted, InboxCreateRequest,
    RegisterDeviceRequest, SyncResponse, UpdateChannelRequest, UpsertAlarmRequest,
    UpsertTodoRequest,
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
        .route("/api/auth/register", post(register_account))
        .route("/api/auth/login", post(login_account))
        .route("/api/auth/logout", post(logout_account))
        .route("/api/auth/me", get(me))
        .route("/api/auth/password", post(change_password))
        .route("/api/admin/accounts", get(admin_list_accounts))
        .route(
            "/api/admin/accounts/:account_id",
            delete(admin_delete_account),
        )
        .route(
            "/api/admin/accounts/:account_id/password",
            post(admin_reset_password),
        )
        .route("/api/sync", get(device_sync).post(device_push_sync))
        .route("/api/devices", post(register_device).get(list_devices))
        .route("/api/devices/:device_id", delete(delete_device))
        .route(
            "/api/devices/:device_id/channels",
            get(list_channels).post(create_channel),
        )
        .route(
            "/api/devices/:device_id/channels/:channel_id",
            get(get_channel).put(update_channel).delete(delete_channel),
        )
        .route(
            "/api/devices/:device_id/channels/:channel_id/rotate-token",
            post(rotate_channel_token),
        )
        .route("/api/channels/:channel_id/messages", post(deliver_inbox))
        .route(
            "/api/devices/:device_id/inbox",
            get(list_inbox).delete(clear_inbox),
        )
        .route(
            "/api/devices/:device_id/inbox/:seq",
            delete(delete_inbox_item),
        )
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

// --- Console accounts ------------------------------------------------------

async fn register_account(State(state): State<AppState>, Json(req): Json<AuthRequest>) -> Response {
    let username = req.username.trim();
    if let Err(msg) = crate::auth::validate_username(username) {
        return bad_request(msg);
    }
    if let Err(msg) = crate::auth::validate_password(&req.password) {
        return bad_request(msg);
    }
    if match db::find_account_by_username(&state.db, username).await {
        Ok(v) => v,
        Err(err) => return internal_error(err),
    }
    .is_some()
    {
        return (StatusCode::CONFLICT, "username already taken").into_response();
    }
    let hash = match crate::auth::hash_password(&req.password) {
        Ok(h) => h,
        Err(err) => return internal_error(err),
    };
    let account = match db::register_account(&state.db, username, &hash).await {
        Ok(a) => a,
        Err(err) => return internal_error(err),
    };
    issue_session(&state, account).await
}

async fn login_account(State(state): State<AppState>, Json(req): Json<AuthRequest>) -> Response {
    let username = req.username.trim();
    let stored = match db::find_account_by_username(&state.db, username).await {
        Ok(v) => v,
        Err(err) => return internal_error(err),
    };
    let account_id = match stored {
        Some((id, hash)) => {
            if !crate::auth::verify_password(&req.password, &hash) {
                return (StatusCode::UNAUTHORIZED, "invalid username or password").into_response();
            }
            id
        }
        None => {
            // Verify against a throwaway hash so unknown usernames cost a
            // real Argon2 round too - no easy user enumeration by timing.
            let _ = crate::auth::verify_password(
                &req.password,
                "$argon2id$v=19$m=19456,t=2,p=1$AAAAAAAAAAAAAAAAAAAAAA$AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
            );
            return (StatusCode::UNAUTHORIZED, "invalid username or password").into_response();
        }
    };
    let account = match db::account_by_id(&state.db, account_id).await {
        Ok(Some(a)) => a,
        Ok(None) => {
            return (StatusCode::UNAUTHORIZED, "invalid username or password").into_response()
        }
        Err(err) => return internal_error(err),
    };
    issue_session(&state, account).await
}

async fn issue_session(state: &AppState, account: Account) -> Response {
    let token = match db::create_session(&state.db, account.id).await {
        Ok(t) => t,
        Err(err) => return internal_error(err),
    };
    tracing::info!(account_id = account.id, username = %account.username, "console session issued");
    (
        StatusCode::OK,
        Json(AuthResponse {
            token,
            username: account.username,
        }),
    )
        .into_response()
}

async fn logout_account(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Some(token) = bearer_token(&headers) else {
        return (StatusCode::NO_CONTENT).into_response();
    };
    match db::delete_session(&state.db, token).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(err) => internal_error(err),
    }
}

/// Validates a stored session token so the console can confirm it's still
/// logged in on load. Also answers for the admin token.
async fn me(State(state): State<AppState>, headers: HeaderMap) -> Response {
    match auth_context(&headers, &state).await {
        Ok(AuthContext::Admin) => Json(serde_json::json!({ "kind": "admin" })).into_response(),
        Ok(AuthContext::Account(account_id)) => {
            match db::account_by_id(&state.db, account_id).await {
                Ok(Some(account)) => Json(serde_json::json!({
                    "kind": "account",
                    "account_id": account.id,
                    "username": account.username
                }))
                .into_response(),
                Ok(None) => (StatusCode::UNAUTHORIZED, "invalid session").into_response(),
                Err(err) => internal_error(err),
            }
        }
        Err(resp) => resp.into_response(),
    }
}

async fn change_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ChangePasswordRequest>,
) -> Response {
    let ctx = match auth_context(&headers, &state).await {
        Ok(ctx) => ctx,
        Err(resp) => return resp.into_response(),
    };
    let AuthContext::Account(account_id) = ctx else {
        // The admin token is configured via env, not a password we can
        // change - reject instead of pretending to succeed.
        return (
            StatusCode::BAD_REQUEST,
            "admin token is set in server config",
        )
            .into_response();
    };
    let stored = match db::account_by_id(&state.db, account_id).await {
        Ok(Some(a)) => a,
        Ok(None) => return (StatusCode::UNAUTHORIZED, "invalid session").into_response(),
        Err(err) => return internal_error(err),
    };
    let (_, hash) = match db::find_account_by_username(&state.db, &stored.username).await {
        Ok(Some(v)) => v,
        Ok(None) => return (StatusCode::UNAUTHORIZED, "invalid session").into_response(),
        Err(err) => return internal_error(err),
    };
    if !crate::auth::verify_password(&req.old_password, &hash) {
        return (StatusCode::UNAUTHORIZED, "current password is incorrect").into_response();
    }
    if let Err(msg) = crate::auth::validate_password(&req.new_password) {
        return bad_request(msg);
    }
    let new_hash = match crate::auth::hash_password(&req.new_password) {
        Ok(h) => h,
        Err(err) => return internal_error(err),
    };
    match db::update_account_password(&state.db, account_id, &new_hash).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(err) => internal_error(err),
    }
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

/// Who is calling: the owner's `ADMIN_TOKEN` (full access), a console
/// account session (`account_id`, scoped to that account's devices), or
/// nobody (rejected with 401).
#[derive(Clone, Copy)]
enum AuthContext {
    Admin,
    Account(i64),
}

async fn auth_context(
    headers: &HeaderMap,
    state: &AppState,
) -> Result<AuthContext, (StatusCode, &'static str)> {
    let Some(token) = bearer_token(headers) else {
        return Err((StatusCode::UNAUTHORIZED, "missing or invalid credentials"));
    };
    if token == state.admin_token {
        return Ok(AuthContext::Admin);
    }
    match db::find_session(&state.db, token).await {
        Ok(Some(account_id)) => Ok(AuthContext::Account(account_id)),
        Ok(None) => Err((StatusCode::UNAUTHORIZED, "missing or invalid credentials")),
        Err(err) => {
            tracing::error!("{err:#}");
            Err((StatusCode::INTERNAL_SERVER_ERROR, "storage error"))
        }
    }
}

/// Authenticates and, for account sessions, verifies the account owns
/// `device_id`. The admin token may access any device. Returns the resolved
/// context so the caller knows whether it is managing another account's
/// device (relevant for `register_device`, which must attach ownership).
async fn require_device_access(
    headers: &HeaderMap,
    state: &AppState,
    device_id: &str,
) -> Result<AuthContext, Box<Response>> {
    let ctx = auth_context(headers, state)
        .await
        .map_err(|e| Box::new(e.into_response()))?;
    match ctx {
        AuthContext::Admin => Ok(ctx),
        AuthContext::Account(account_id) => {
            match db::device_owned_by(&state.db, device_id, account_id).await {
                Ok(true) => Ok(ctx),
                Ok(false) => Err(Box::new(
                    (StatusCode::NOT_FOUND, "device not found").into_response(),
                )),
                Err(err) => Err(Box::new(internal_error(err))),
            }
        }
    }
}

fn internal_error(err: anyhow::Error) -> Response {
    tracing::error!("{err:#}");
    (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response()
}

// --- Admin: account management ---------------------------------------------

/// The `ADMIN_TOKEN` is the owner credential; account sessions are not
/// allowed past this gate.
async fn require_admin_only(
    headers: &HeaderMap,
    state: &AppState,
) -> Result<(), (StatusCode, &'static str)> {
    match auth_context(headers, state).await {
        Ok(AuthContext::Admin) => Ok(()),
        Ok(AuthContext::Account(_)) => Err((StatusCode::FORBIDDEN, "admin token required")),
        Err(resp) => Err(resp),
    }
}

async fn admin_list_accounts(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(resp) = require_admin_only(&headers, &state).await {
        return resp.into_response();
    }
    match db::list_accounts(&state.db).await {
        Ok(accounts) => Json(accounts).into_response(),
        Err(err) => internal_error(err),
    }
}

async fn admin_delete_account(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(account_id): Path<i64>,
) -> Response {
    if let Err(resp) = require_admin_only(&headers, &state).await {
        return resp.into_response();
    }
    match db::delete_account(&state.db, account_id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => (StatusCode::NOT_FOUND, "account not found").into_response(),
        Err(err) => internal_error(err),
    }
}

async fn admin_reset_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(account_id): Path<i64>,
    Json(req): Json<AdminResetPasswordRequest>,
) -> Response {
    if let Err(resp) = require_admin_only(&headers, &state).await {
        return resp.into_response();
    }
    if let Err(msg) = crate::auth::validate_password(&req.new_password) {
        return bad_request(msg);
    }
    let hash = match crate::auth::hash_password(&req.new_password) {
        Ok(h) => h,
        Err(err) => return internal_error(err),
    };
    match db::update_account_password(&state.db, account_id, &hash).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(err) => internal_error(err),
    }
}

// --- Device-facing sync endpoint (docs/sync-api.md) -------------------

/// Builds the sync payload (alarms/todos + capped inbox). Reuses the inbox
/// read-merge so the `inbox_read` upload is folded into the response.
async fn build_sync_response(
    state: &AppState,
    device_id: &str,
    inbox_read: &[u64],
) -> Result<SyncResponse, Response> {
    let alarms = db::list_alarms(&state.db, device_id)
        .await
        .map_err(internal_error)?;
    let todos = db::list_todos(&state.db, device_id)
        .await
        .map_err(internal_error)?;
    let acked = db::mark_inbox_read(&state.db, device_id, inbox_read)
        .await
        .map_err(internal_error)?;
    let (inbox, truncated) = db::list_inbox(&state.db, device_id, INBOX_LIMIT)
        .await
        .map_err(internal_error)?;
    Ok(SyncResponse {
        alarms,
        todos,
        inbox,
        inbox_read_acked: acked,
        inbox_truncated: truncated,
    })
}

/// Max inbox items sent to the device in one sync response (hard capacity).
const INBOX_LIMIT: usize = 20;

async fn device_sync(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Some(token) = bearer_token(&headers) else {
        tracing::warn!("sync rejected: missing bearer token");
        return (StatusCode::UNAUTHORIZED, "missing bearer token").into_response();
    };
    let (device_id, version) = match db::find_device_by_token(&state.db, token).await {
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

    let body = match build_sync_response(&state, &device_id, &[]).await {
        Ok(body) => Json(body),
        Err(resp) => return resp.into_response(),
    };
    tracing::info!(
        device_id,
        version,
        alarm_count = body.0.alarms.len(),
        todo_count = body.0.todos.len(),
        inbox_count = body.0.inbox.len(),
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
    let device_id = match db::find_device_by_token(&state.db, token).await {
        Ok(Some((id, _))) => id,
        Ok(None) => return (StatusCode::UNAUTHORIZED, "unknown device token").into_response(),
        Err(err) => return internal_error(err),
    };
    let version = match db::merge_device_state(&state.db, &device_id, &req).await {
        Ok(version) => version,
        Err(err) => return internal_error(err),
    };

    // Long-poll: when the device asks with `X-Inkpaper-Wait`, hold the
    // connection (polling every 500ms) until an unread high-priority inbox
    // message arrives or the timeout elapses, so urgent messages surface
    // in real time without the device hammering the server on a timer. The
    // device keeps one connection open (Wi-Fi stays connected), which is
    // both more real-time and gentler on the radio than repeated connects.
    let wants_wait = headers
        .get("x-inkpaper-wait")
        .and_then(|v| v.to_str().ok())
        .map(|s| s == "1" || s.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    if wants_wait {
        let wait_secs = LONG_POLL_TIMEOUT_SECS;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(wait_secs);
        loop {
            let has_urgent = match db::has_unread_high_inbox(&state.db, &device_id).await {
                Ok(v) => v,
                Err(err) => return internal_error(err),
            };
            if has_urgent {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    }

    let body = match build_sync_response(&state, &device_id, &req.inbox_read).await {
        Ok(body) => Json(body),
        Err(resp) => return resp.into_response(),
    };
    let etag = format!("\"d{device_id}-v{version}\"");
    tracing::info!(
        device_id,
        version,
        wait = wants_wait,
        alarm_count = body.0.alarms.len(),
        todo_count = body.0.todos.len(),
        inbox_count = body.0.inbox.len(),
        "device state merged and sync payload served"
    );
    (StatusCode::OK, [(axum::http::header::ETAG, etag)], body).into_response()
}

/// How long a long-polling sync request holds the connection waiting for an
/// urgent message before returning an empty wait (timeout). Kept modest so a
/// hung device can't pin a connection forever.
const LONG_POLL_TIMEOUT_SECS: u64 = 30;

// --- Admin: devices -----------------------------------------------------

async fn register_device(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<RegisterDeviceRequest>,
) -> Response {
    let ctx = match auth_context(&headers, &state).await {
        Ok(ctx) => ctx,
        Err(resp) => return resp.into_response(),
    };
    if req.name.trim().is_empty() || req.name.chars().count() > 80 {
        return bad_request("device name must be 1..80 characters");
    }
    let account_id = match ctx {
        AuthContext::Admin => None,
        AuthContext::Account(id) => Some(id),
    };
    match db::register_device(&state.db, &req.name, account_id).await {
        Ok(device) => (StatusCode::CREATED, Json(device)).into_response(),
        Err(err) => internal_error(err),
    }
}

async fn list_devices(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let ctx = match auth_context(&headers, &state).await {
        Ok(ctx) => ctx,
        Err(resp) => return resp.into_response(),
    };
    let devices = match ctx {
        AuthContext::Admin => db::list_devices(&state.db).await,
        AuthContext::Account(account_id) => db::list_account_devices(&state.db, account_id).await,
    };
    match devices {
        Ok(devices) => Json(devices).into_response(),
        Err(err) => internal_error(err),
    }
}

async fn delete_device(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(device_id): Path<String>,
) -> Response {
    match require_device_access(&headers, &state, &device_id).await {
        Ok(_) => {}
        Err(resp) => return *resp,
    }
    match db::delete_device(&state.db, &device_id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(err) => internal_error(err),
    }
}

// --- Admin: alarms --------------------------------------------------------

async fn list_alarms(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(device_id): Path<String>,
) -> Response {
    if let Err(resp) = require_device_access(&headers, &state, &device_id).await {
        return *resp;
    }
    match db::list_alarms(&state.db, &device_id).await {
        Ok(alarms) => Json(alarms).into_response(),
        Err(err) => internal_error(err),
    }
}

async fn create_alarm(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(device_id): Path<String>,
    Json(req): Json<UpsertAlarmRequest>,
) -> Response {
    if let Err(resp) = require_device_access(&headers, &state, &device_id).await {
        return *resp;
    }
    if let Err(msg) = validate_alarm(&req) {
        return bad_request(msg);
    }
    match db::upsert_alarm(&state.db, &device_id, None, &req).await {
        Ok(id) => (StatusCode::CREATED, Json(serde_json::json!({ "id": id }))).into_response(),
        Err(err) => internal_error(err),
    }
}

async fn update_alarm(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((device_id, alarm_id)): Path<(String, u8)>,
    Json(req): Json<UpsertAlarmRequest>,
) -> Response {
    if let Err(resp) = require_device_access(&headers, &state, &device_id).await {
        return *resp;
    }
    if let Err(msg) = validate_alarm(&req) {
        return bad_request(msg);
    }
    match db::upsert_alarm(&state.db, &device_id, Some(alarm_id), &req).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(err) => internal_error(err),
    }
}

async fn clear_alarms(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(device_id): Path<String>,
) -> Response {
    if let Err(resp) = require_device_access(&headers, &state, &device_id).await {
        return *resp;
    }
    match db::clear_alarms(&state.db, &device_id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(err) => internal_error(err),
    }
}

async fn delete_alarm(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((device_id, alarm_id)): Path<(String, u8)>,
) -> Response {
    if let Err(resp) = require_device_access(&headers, &state, &device_id).await {
        return *resp;
    }
    match db::delete_alarm(&state.db, &device_id, alarm_id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(err) => internal_error(err),
    }
}

// --- Admin: todos ---------------------------------------------------------

async fn list_todos(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(device_id): Path<String>,
) -> Response {
    if let Err(resp) = require_device_access(&headers, &state, &device_id).await {
        return *resp;
    }
    match db::list_todos(&state.db, &device_id).await {
        Ok(todos) => Json(todos).into_response(),
        Err(err) => internal_error(err),
    }
}

async fn create_todo(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(device_id): Path<String>,
    Json(req): Json<UpsertTodoRequest>,
) -> Response {
    if let Err(resp) = require_device_access(&headers, &state, &device_id).await {
        return *resp;
    }
    if let Err(msg) = validate_todo(&req) {
        return bad_request(msg);
    }
    match db::upsert_todo(&state.db, &device_id, None, &req).await {
        Ok(id) => (StatusCode::CREATED, Json(serde_json::json!({ "id": id }))).into_response(),
        Err(err) => internal_error(err),
    }
}

async fn update_todo(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((device_id, todo_id)): Path<(String, u8)>,
    Json(req): Json<UpsertTodoRequest>,
) -> Response {
    if let Err(resp) = require_device_access(&headers, &state, &device_id).await {
        return *resp;
    }
    if let Err(msg) = validate_todo(&req) {
        return bad_request(msg);
    }
    match db::upsert_todo(&state.db, &device_id, Some(todo_id), &req).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(err) => internal_error(err),
    }
}

async fn clear_todos(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(device_id): Path<String>,
) -> Response {
    if let Err(resp) = require_device_access(&headers, &state, &device_id).await {
        return *resp;
    }
    match db::clear_todos(&state.db, &device_id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(err) => internal_error(err),
    }
}

async fn delete_todo(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((device_id, todo_id)): Path<(String, u8)>,
) -> Response {
    if let Err(resp) = require_device_access(&headers, &state, &device_id).await {
        return *resp;
    }
    match db::delete_todo(&state.db, &device_id, todo_id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(err) => internal_error(err),
    }
}

// --- External channels & inbox --------------------------------------------

fn validate_channel_request(req: &CreateChannelRequest) -> Result<(), &'static str> {
    if req.name.trim().is_empty() || req.name.chars().count() > 80 {
        return Err("channel name must be 1..80 characters");
    }
    match req.kind.as_str() {
        "webhook" => Ok(()),
        other => Err(match other {
            "caldav_basic" => "caldav_basic is not yet implemented",
            _ => "channel kind must be 'webhook'",
        }),
    }
}

/// Validates a webhook payload; rejects (rather than silently truncates)
/// anything that violates the size limits.
fn validate_inbox_payload(req: &InboxCreateRequest) -> Result<(), &'static str> {
    if !matches!(req.kind.as_str(), "alert" | "event" | "info") {
        return Err("kind must be one of 'alert' | 'event' | 'info'");
    }
    let title = req.title.trim();
    if title.is_empty() {
        return Err("title must not be empty");
    }
    if req.title.chars().count() > 120 || req.title.len() > 512 {
        return Err("title too long (max 120 chars / 512 bytes)");
    }
    if req.body.chars().count() > 1000 || req.body.len() > 4096 {
        return Err("body too long (max 1000 chars / 4 KiB)");
    }
    if let Some(when) = req.when {
        if !(0..=4_102_444_800).contains(&when) {
            return Err("when is outside a valid Unix epoch range");
        }
    }
    if let Some(priority) = req.priority {
        if !matches!(
            priority,
            crate::models::Priority::Normal | crate::models::Priority::High
        ) {
            return Err("priority must be one of 'normal' | 'high'");
        }
    }
    Ok(())
}

async fn list_channels(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(device_id): Path<String>,
) -> Response {
    if let Err(resp) = require_device_access(&headers, &state, &device_id).await {
        return *resp;
    }
    match db::list_channels(&state.db, &device_id).await {
        Ok(channels) => Json(channels).into_response(),
        Err(err) => internal_error(err),
    }
}

async fn create_channel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(device_id): Path<String>,
    Json(req): Json<CreateChannelRequest>,
) -> Response {
    if let Err(resp) = require_device_access(&headers, &state, &device_id).await {
        return *resp;
    }
    if let Err(msg) = validate_channel_request(&req) {
        return bad_request(msg);
    }
    // Phase 1 is webhook-only; no CalDAV config is accepted yet.
    let config_encrypted = None;
    match db::create_channel(
        &state.db,
        &device_id,
        &req.kind,
        &req.name,
        config_encrypted,
    )
    .await
    {
        Ok((channel, token)) => {
            let delivery_url = token
                .as_ref()
                .map(|_| format!("/api/channels/{}/messages", channel.id));
            (
                StatusCode::CREATED,
                Json(ChannelCreated {
                    channel,
                    token,
                    delivery_url,
                }),
            )
                .into_response()
        }
        Err(err) => internal_error(err),
    }
}

async fn get_channel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((device_id, channel_id)): Path<(String, String)>,
) -> Response {
    if let Err(resp) = require_device_access(&headers, &state, &device_id).await {
        return *resp;
    }
    match db::get_channel(&state.db, &device_id, &channel_id).await {
        Ok(Some(channel)) => Json(channel).into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, "channel not found").into_response(),
        Err(err) => internal_error(err),
    }
}

async fn update_channel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((device_id, channel_id)): Path<(String, String)>,
    Json(req): Json<UpdateChannelRequest>,
) -> Response {
    if let Err(resp) = require_device_access(&headers, &state, &device_id).await {
        return *resp;
    }
    let name = req.name.as_deref();
    if let Some(name) = name {
        if name.trim().is_empty() || name.chars().count() > 80 {
            return bad_request("channel name must be 1..80 characters");
        }
    }
    match db::update_channel(&state.db, &device_id, &channel_id, name, req.enabled).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => (StatusCode::NOT_FOUND, "channel not found").into_response(),
        Err(err) => internal_error(err),
    }
}

async fn delete_channel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((device_id, channel_id)): Path<(String, String)>,
) -> Response {
    if let Err(resp) = require_device_access(&headers, &state, &device_id).await {
        return *resp;
    }
    match db::delete_channel(&state.db, &device_id, &channel_id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => (StatusCode::NOT_FOUND, "channel not found").into_response(),
        Err(err) => internal_error(err),
    }
}

async fn rotate_channel_token(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((device_id, channel_id)): Path<(String, String)>,
) -> Response {
    if let Err(resp) = require_device_access(&headers, &state, &device_id).await {
        return *resp;
    }
    match db::rotate_channel_token(&state.db, &device_id, &channel_id).await {
        Ok(Some(token)) => {
            let prefix = token.chars().take(12).collect::<String>();
            Json(serde_json::json!({ "token": token, "token_prefix": prefix })).into_response()
        }
        Ok(None) => (StatusCode::NOT_FOUND, "channel not found or not a webhook").into_response(),
        Err(err) => internal_error(err),
    }
}

/// Webhook delivery endpoint: authenticates via the channel's own bearer
/// token (distinct from admin/device tokens), validates the payload, and
/// inserts an inbox item. `Idempotency-Key` maps to `source_ref`.
async fn deliver_inbox(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(channel_id): Path<String>,
    Json(req): Json<InboxCreateRequest>,
) -> Response {
    // Query the channel by id first, then verify the token. Both wrong-token
    // and unknown-channel end as 401/404 so channel existence isn't leaked.
    let Some((device_id, token_hash)) =
        (match db::get_channel_for_delivery(&state.db, &channel_id).await {
            Ok(v) => v,
            Err(err) => return internal_error(err),
        })
    else {
        return (StatusCode::NOT_FOUND, "unknown channel").into_response();
    };
    let Some(token) = bearer_token(&headers) else {
        return (StatusCode::UNAUTHORIZED, "invalid channel token").into_response();
    };
    if !crate::auth::verify_password(token, &token_hash) {
        return (StatusCode::UNAUTHORIZED, "invalid channel token").into_response();
    }
    // Only enabled channels accept deliveries.
    let enabled = match db::get_channel(&state.db, &device_id, &channel_id).await {
        Ok(Some(c)) => c.enabled,
        _ => return (StatusCode::NOT_FOUND, "unknown channel").into_response(),
    };
    if !enabled {
        return (StatusCode::FORBIDDEN, "channel disabled").into_response();
    }
    if let Err(msg) = validate_inbox_payload(&req) {
        return bad_request(msg);
    }
    let source_ref = headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let priority = req.priority.unwrap_or(crate::models::Priority::Normal);
    match db::deliver_inbox(
        &state.db,
        &device_id,
        &channel_id,
        req.kind.as_str(),
        match priority {
            crate::models::Priority::High => "high",
            crate::models::Priority::Normal => "normal",
        },
        req.title.trim(),
        req.body.trim(),
        req.when,
        source_ref.as_deref(),
    )
    .await
    {
        Ok((seq, created)) => {
            let status = if created {
                StatusCode::CREATED
            } else {
                StatusCode::OK
            };
            (
                status,
                Json(InboxAccepted {
                    accepted: true,
                    id: seq,
                }),
            )
                .into_response()
        }
        Err(err) => internal_error(err),
    }
}

async fn list_inbox(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(device_id): Path<String>,
) -> Response {
    if let Err(resp) = require_device_access(&headers, &state, &device_id).await {
        return *resp;
    }
    match db::list_inbox(&state.db, &device_id, 200).await {
        Ok((items, _truncated)) => Json(items).into_response(),
        Err(err) => internal_error(err),
    }
}

async fn delete_inbox_item(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((device_id, seq)): Path<(String, u64)>,
) -> Response {
    if let Err(resp) = require_device_access(&headers, &state, &device_id).await {
        return *resp;
    }
    match db::delete_inbox_item(&state.db, &device_id, seq).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => (StatusCode::NOT_FOUND, "inbox item not found").into_response(),
        Err(err) => internal_error(err),
    }
}

async fn clear_inbox(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(device_id): Path<String>,
) -> Response {
    if let Err(resp) = require_device_access(&headers, &state, &device_id).await {
        return *resp;
    }
    match db::clear_read_inbox(&state.db, &device_id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(err) => internal_error(err),
    }
}
