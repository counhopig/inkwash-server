# AGENTS.md

Personal-scale device-cloud backend in Rust + axum: an admin API guarded by the single `ADMIN_TOKEN` **plus** console-account sessions (`POST /api/auth/*`, Argon2id passwords, per-account device ownership), and a device-facing sync endpoint `/api/sync` (per-device tokens, ETag/304 caching). The Vue 3 console `admin-ui/` is embedded into the binary at compile time via rust-embed. One of three repos: firmware + protocol docs live in `../inkpaper`, the desktop tool in `../inkpaper-desktop`.

## Critical gotchas

- `build.rs` runs `npm run build` (in `admin-ui/`) on every `cargo build`/`cargo run` and panics if `admin-ui/node_modules` is missing - run `npm install --prefix admin-ui` once after cloning. UI changes are rebuilt automatically; before touching Rust, make sure node_modules still exists.
- `ADMIN_TOKEN` is required or the server refuses to start; `.env` is loaded automatically by dotenvy from the working directory and is gitignored (never commit keys). Also `DATABASE_PATH` (default `inkpaper.sqlite3`) and `BIND_ADDR` (default `0.0.0.0:8080`).
- No test CI. Pre-commit verification: `cargo fmt --check && cargo clippy --all-targets && cargo test` (only the 2 unit tests in `db.rs`) plus `npm run build` in `admin-ui/` (= `vue-tsc --noEmit` + vite build). The only workflow is `.github/workflows/release.yml` (below).

## Releases

`.github/workflows/release.yml` builds Linux/macOS x86_64 binaries and **publishes automatically** (`draft: false`) whenever a `v*` tag is pushed to the `github` remote — no manual draft handling.

- **Critical:** GitHub Actions runs the workflow file **at the tagged commit**, not at `main`. If you re-trigger a release by deleting + re-pushing a tag, the tag must point to a commit that already contains the latest workflow changes — otherwise the *old* workflow runs. Also delete the old release + remote tag first, because force-updating an existing tag does not reliably re-trigger the workflow:
  ```bash
  gh release delete v0.1.0 --repo counhopig/inkpaper-server --yes
  git push github :refs/tags/v0.1.0
  git tag -f v0.1.0 <commit-with-latest-workflow>
  git push github v0.1.0
  ```
- Release check: `gh release view v0.1.0 --repo counhopig/inkpaper-server --json isDraft,assets` (expect `isDraft: false`, one asset per platform).

## Architecture

- Entry chain: `src/main.rs` -> `src/routes.rs` (axum router; handlers return a concrete `Response`, auth is manual via a `HeaderMap` parameter) -> `src/db.rs` (SQLite behind a single shared `Arc<Mutex<Connection>>` - deliberately no connection pool; don't switch to sqlx/deadpool) -> `src/models.rs` (wire types). `src/auth.rs` has the Argon2id password hashing + username/password validation.
- Three trust domains share one router:
  - `ADMIN_TOKEN` (env, from `main.rs`) - full access to every device, including unowned ones. This is how `inkpaper-desktop` keeps working unchanged.
  - Console-account sessions (`/api/auth/login` / `/api/auth/register` return a bearer session token stored in the `sessions` table) - scoped to that account's own devices (`devices.account_id`); touching another account's device returns 404. `auth_context()` in `routes.rs` resolves which domain a request is in; `require_device_access()` additionally checks ownership for account sessions.
  - `/api/sync` is authenticated per-device by the token `register_device` issued.
- Every alarm/todo mutation bumps `devices.version`; `GET /api/sync` returns ETag `"d{device_id}-v{version}"` and answers 304 on matching `If-None-Match` (device id is embedded so a stale cache from an old/re-registered device can't suppress the first payload).
- `POST /api/sync`: the device may only upload `enabled` flags for alarms and `done` flags for todos; unknown ids are silently ignored (no recreating content deleted server-side).
- Alarm/todo `local_id` is `u8` (0..255); new ids take MAX+1 and error at the limit (`next_local_id`).
- Accounts/sessions live in `accounts` and `sessions` tables; `devices.account_id` is `NULL` for devices registered with the admin token (desktop tool) and set for console-registered devices.

## Wire contract (don't change casually)

- `src/models.rs` field names/enum shapes must match the firmware's `StoredAlarm`/`Repeat` (`rust-firmware/src/alarms.rs`) and `Todo` (`rust-firmware/src/todos.rs`) exactly - the firmware deserializes the sync JSON directly, no adapter layer. `Repeat` is serde externally tagged: `"Daily"` or `{"Once":{year,month,day}}` - this is why the UI sends `repeat: "Daily"` (capital D). Any wire-shape change requires updating `../inkpaper/docs/sync-api.md` too.
- A device token is returned exactly once, in the `POST /api/devices` registration response; it is never readable again (`list_devices` omits it) - losing it means re-registering.

## admin-ui

- Vue 3, no router/state library - views switch via a `view` ref in `App.vue` (`dashboard` | `device` | `account`). `App.vue` is the coordinator: it owns the session, device list + per-device stats, and global toast/confirm/busy plumbing, and provides them to child views through `lib/ui.ts` (an `InjectionKey`; views `inject(uiKey)` instead of prop-drilling).
- View components: `LoginView.vue` (sign in / create account / admin-token fallback), `DashboardView.vue` (stat strip + device cards + register device), `DeviceView.vue` (alarms/todos editor for one device, loads its own content), `AccountView.vue` (session info, change password, sign out). `src/lib/api.ts` is a thin same-origin fetch wrapper using relative paths - there is no base-URL config. `src/lib/storage.ts` persists the session (`inkpaper.admin.session`) and the admin token.
- Every mutation goes through `ui.run(name, fn)` (double-submit guard + shared 401/error handling; a 401 anywhere drops the session back to login); `loadContent`/`loadDevices` use a sequence guard so stale responses can't overwrite newer ones.
- Dev loop: `npm run dev` (:5173, vite proxies `/api` and `/health` to :8080) alongside a separate `cargo run`; `src/lib/clipboard.ts` falls back to `execCommand('copy')` because the console is normally opened over plain LAN http (no secure context).

## Other

- `scripts/start.sh` does the full first-run setup (generates `.env` with a fresh `ADMIN_TOKEN`, runs `npm install`, then `cargo run --release`).
- Commits use Conventional Commits with English subjects (`feat:`/`fix:`/`docs:`/`chore:`/`build:`).
