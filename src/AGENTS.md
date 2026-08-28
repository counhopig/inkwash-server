# AGENTS.md — inkwash-server/src

## OVERVIEW
Rust crate root: 8 files, ~153 symbols — the axum HTTP surface, dual-dialect sqlx storage, wire DTOs, and auth helpers the parent describes, with concrete locations.

## WHERE TO LOOK
| File | Role | Edit here when… |
|---|---|---|
| main.rs (60L) | Env bootstrap: `DATABASE_URL` (sqlite default / `postgres://` switch), `ADMIN_TOKEN` (required, main.rs:34-38), `BIND_ADDR`; `db::open(&db_url, 2)` (:44), `routes::router(state).layer(TraceLayer)` (:54). Startup log prints `db={db_kind}` only (:56) | touching startup/env handling |
| routes.rs (~1260L, ~57 fns) | Whole HTTP surface in one file. `router()` (:48) is the only route-registration point; handlers take `State<AppState>` + `HeaderMap` (+ Path/Json) and return concrete `Response`; shared helpers `bad_request` (:295), `internal_error` (:438), `bearer_token` (:324). Auth dispatch: `authenticate` (:360) → `AuthSubject` (:339); `require_device_access` (:411), `require_admin_only` (:447). Sync: ETag `"d{device_id}-v{version}"` via `sync_etag` (:543), 304 on If-None-Match (:560-563), `INBOX_LIMIT` 20 (:539). Tests module at bottom | adding/editing any route, handler, or validation |
| db.rs (624L) | Module root: `Db` (AnyPool + `postgres` flag), `Db::sql(sqlite, postgres)` picker; re-exports `queries`/`schema`/`sync` so callers keep `db::` paths | the pool/`Db` abstraction itself |
| db/queries.rs (~1124L, 52 fns) | All CRUD: devices, accounts/sessions, channels/inbox, alarms/todos. Every placeholder statement is a SQLite/Postgres pair, integers bound `i64`. `bump_version` (:166) must share the write's transaction (see `upsert_alarm` :281); `next_local_id` MAX+1 u8 alloc (:260). Token storage decision at :53-62: device/session tokens plaintext, channel tokens `ipwh_` + Argon2id (:594, :778) | adding/editing any SQL |
| db/schema.rs | Per-dialect sqlx migrations embedded (`migrations/sqlite` vs `migrations/postgres`); `SQLITE_TABLES` const kept in sync with `0001_init.sql` for the legacy rebuild; SQLite-only `migrate_legacy_integer_ids` + `backfill_missing_columns` with explicit column checks | schema/migration changes |
| db/sync.rs (72L) | `merge_device_state` (:14): the only path where device uploads edit server state — alarms `enabled`, todos `done` (+`importance`), one tx, bumps version only when something changed | sync merge logic |
| models.rs (~414L) | Re-exports `inkwash-logic` wire types (:32-34); server DTOs, `#[derive(TS)]` ones export to `admin-ui/src/lib/generated/`; u64/i64 pinned to TS `number` | wire/DTO shape changes |
| auth.rs (53L) | Argon2id hash/verify; username 3..32 `[A-Za-z0-9_-]`, password 8..128 | credential rules |

## CONVENTIONS
- sqlx Any driver + backend flag; every handler `await`s sqlx — no blocking calls.
- Admin token compared constant-time via `subtle`; no auth path outside `AuthSubject` (routes.rs:339).
- New query = dual-dialect `Db::sql` pair: same tables/columns/bind order, only `?` vs `$1..` differs — no runtime rewriting.
- Version-bumping writes share one transaction with `bump_version`; a failed write must not leave a phantom bump.

## ANTI-PATTERNS
- Logging/printing `DATABASE_URL` — may embed postgres credentials; the comment at main.rs:24-26 says so, and the startup log prints only `db={db_kind}` (main.rs:56). Keep it that way.
- "Fixing" the silent-ignore of unknown ids in POST /api/sync and `mark_inbox_read` — deliberate, so stale device caches can't recreate server-deleted content (db/sync.rs:11, queries.rs:978).
- Bolting auth onto a handler instead of `authenticate()` (routes.rs:360) — that's how a fifth credential kind sneaks in.
- Swallowing DB errors (`let _ = …`) in schema backfills — `backfill_missing_columns` existence-checks each column and propagates failures loudly instead.
