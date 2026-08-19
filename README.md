# Inkpaper Server

Personal-scale cloud backend for the **Zectrix Note 4** e-ink device —
stores alarms and todos per device and serves them over the sync contract.
One half of the [**Inkpaper**](https://github.com/counhopig/inkpaper-firmware)
ecosystem, alongside a PC tool and the device firmware.

[![License](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-edition%202021-orange.svg)](Cargo.toml)
[![Platform](https://img.shields.io/badge/Platform-Linux%2FmacOS%2FWindows-lightgrey.svg)]()

## What it is

A small, **personal** cloud server for one user's devices — deliberately
not multi-tenant. A single shared admin bearer token guards device
registration and content management; each registered device gets its own
bearer token for the sync endpoint. The admin console (a Vue 3 app) is
compiled into the binary, so the server is a single deployable artifact.

```mermaid
flowchart LR
    D[Zectrix Note 4<br/>inkpaper-firmware] -->|POST /api/sync<br/>done/enabled flags| S[inkpaper-server]
    S -->"alarms + todos JSON" D
    T[inkpaper-desktop] -->|admin API / ADMIN_TOKEN| S
    U[Browser] -->|embedded admin UI| S
```

## Run

Easiest path — [`scripts/start.sh`](scripts/start.sh) generates a `.env`
with a random `ADMIN_TOKEN`, runs `npm install` in `admin-ui/`, then
launches:

```bash
./scripts/start.sh
```

Manual setup:

```bash
printf 'ADMIN_TOKEN=%s\n' "$(openssl rand -hex 32)" > .env
npm install --prefix admin-ui
cargo run --release
```

Env vars:

| Var            | Default               | Purpose                                   |
| -------------- | --------------------- | ----------------------------------------- |
| `ADMIN_TOKEN`  | _(required)_          | Bearer token for the admin API / console  |
| `DATABASE_PATH`| `inkpaper.sqlite3`    | SQLite file location                      |
| `BIND_ADDR`    | `0.0.0.0:8080`        | Listen address                            |

`.env` is loaded from the working directory; existing process
environment takes precedence.

## API

- `GET /health` — unauthenticated liveness check.
- `GET /api/sync` — device-facing (`Bearer <device_token>`), ETag/304
  conditional pull (legacy; kept for older firmware).
- `POST /api/sync` — device-facing (`Bearer <device_token>`): uploads
  `enabled`/`done`/importance flags, merges them (unknown IDs ignored),
  returns the authoritative `{alarms, todos}` list. This is what current
  firmware calls — contract in the firmware repo's
  [`docs/sync-api.md`](https://github.com/counhopig/inkpaper-firmware/blob/main/docs/sync-api.md).
- `POST/GET/DELETE /api/devices[/:id]` — admin, `Bearer <ADMIN_TOKEN>`.
  Registration returns the device token exactly once.
- `/api/devices/:id/alarms` and `/todos` (GET/POST/PUT/DELETE, plus a
  DELETE-to-clear on each collection) — admin, same auth.

Invalid times, dates, empty names/todos and overlong text are rejected
with HTTP 400 before reaching SQLite.

## Tests

```bash
cargo test          # 2 unit tests (DB schema + device-state merge)
cd admin-ui && npm run build   # vue-tsc type check + vite build
```

Exercised end-to-end against the physical device via `inkpaper-desktop`.

## License

[Apache-2.0](LICENSE).
