# Inkpaper Server

Backend for the [Inkpaper NOTE4 firmware](../inkpaper) - stores alarms and
todos per device and serves them over the sync contract documented in
[`../inkpaper/docs/sync-api.md`](../inkpaper/docs/sync-api.md).

Rust + `axum` + `rusqlite` (SQLite, bundled - no system dependency).
Personal-scale by design: one shared admin bearer token guards device
registration and content management (not a multi-tenant service), and
each registered device gets its own bearer token for the read-only sync
endpoint.

## Run

Easiest path - [`scripts/start.sh`](scripts/start.sh) handles first-run
setup (generates `.env` with a random `ADMIN_TOKEN` if missing, runs
`npm install` in `admin-ui/`) and then launches the server:

```bash
./scripts/start.sh
```

`cargo build`/`cargo run` runs `npm run build` in [`admin-ui/`](admin-ui) automatically
(via `build.rs`) and embeds the compiled output into the binary, so the
server stays a single deployable artifact - `npm install` is the only
one-time setup step. See [`admin-ui/README.md`](admin-ui/README.md) for
that app's own dev workflow.

For a manual setup instead:

```bash
printf 'ADMIN_TOKEN=%s\n' "$(openssl rand -hex 32)" > .env
npm install --prefix admin-ui   # one-time: installs the admin console's build tooling
cargo run --release
```

Env vars:

- `ADMIN_TOKEN` (required) - bearer token [`inkpaper-desktop`](../inkpaper-desktop)
  uses for device registration and alarm/todo management. Generate a long
  random string and keep it secret.
- `DATABASE_PATH` (default `inkpaper.sqlite3`) - SQLite file location.
- `BIND_ADDR` (default `0.0.0.0:8080`) - listen address.

The server automatically loads `.env` from its working directory. Values
already present in the process environment take precedence.

## API

Open `/` in a browser for the admin console (`admin-ui/`, a small Vue 3
app embedded into the binary). It can register devices, copy the
one-time device token, and manage alarms and todos using `ADMIN_TOKEN`.

- `GET /health` - unauthenticated liveness check.
- `GET /api/sync` - device-facing, `Authorization: Bearer <device_token>`,
  supports `If-None-Match` (legacy/read-only pull; kept for older firmware).
- `POST /api/sync` - device-facing, `Authorization: Bearer <device_token>`,
  body `{"alarms":[{"id":u8,"enabled":bool}],"todos":[{"id":u8,"done":bool}]}`.
  Merges the device's uploaded `enabled`/`done` flags into the stored alarms/
  todos (unknown IDs are ignored) and returns the same JSON shape as `GET`.
  This is what current firmware actually calls - see `docs/sync-api.md` in
  the firmware repo for the exact contract both endpoints implement.
- `POST /api/devices`, `GET /api/devices`, `DELETE /api/devices/:id` -
  admin, `Authorization: Bearer <ADMIN_TOKEN>`. Registration returns the
  device's token exactly once (not retrievable again afterward).
- `GET/POST /api/devices/:id/alarms`, `PUT/DELETE /api/devices/:id/alarms/:alarm_id`
  and the equivalent `/todos` routes - admin, same auth.
- `DELETE /api/devices/:id/alarms` and `/todos` clear the corresponding
  collection while preserving the device and its other content.

Invalid times, dates, empty names/todos, and overlong user-facing text are
rejected with HTTP 400 before reaching SQLite.

Alarm/todo JSON shapes mirror the firmware's `alarms::StoredAlarm`/
`todos::Todo` types exactly (see `src/models.rs`) - the device deserializes
the sync response directly into those types, no adapter layer.

## Status

Fully tested via a `curl`-driven pass: device registration, alarm/todo
CRUD (both `Daily` and `Once` repeat kinds), the sync endpoint's JSON
shape (verified byte-for-byte against the spec doc's example), ETag
caching (200 then 304), and auth rejection on both surfaces. Also
exercised for real against the physical device via `inkpaper-desktop`.
See [`../inkpaper/docs/project-status.md`](../inkpaper/docs/project-status.md)
for the full cross-repo status.
