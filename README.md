# Inkpaper Server

Personal-scale cloud backend for the **Zectrix Note 4** e-ink device —
stores alarms and todos per device and serves them over the sync contract.
One half of the [**Inkpaper**](https://github.com/counhopig/inkpaper-firmware)
ecosystem, alongside a PC tool and the device firmware.

[![License](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-edition%202021-orange.svg)](Cargo.toml)
[![Platform](https://img.shields.io/badge/Platform-Linux%2FmacOS%2FWindows-lightgrey.svg)]()

## What it is

A small, **personal** cloud server for a handful of devices. The owner
signs in with a single `ADMIN_TOKEN` (full access to every device and
every account); other people can register console **accounts** and manage
**their own** devices. Each registered device gets its own bearer token
for the sync endpoint. The admin console (a Vue 3 app) is compiled into
the binary, so the server is a single deployable artifact.

```mermaid
flowchart LR
    D["Zectrix Note 4<br/>inkpaper-firmware"] -->|"POST /api/sync (done/enabled flags)"| S["inkpaper-server<br/>Rust + axum + SQLite"]
    S -->|"JSON alarms + todos"| D
    T["inkpaper-desktop"] -->|"HTTPS admin API (ADMIN_TOKEN)"| S
    U["Browser"] -->|"login /api/auth/* (session or ADMIN_TOKEN)"| S
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
| `ADMIN_TOKEN`  | _(required)_          | Owner bearer token: full access to all devices & accounts |
| `DATABASE_PATH`| `inkpaper.sqlite3`    | SQLite file location                      |
| `BIND_ADDR`    | `0.0.0.0:8080`        | Listen address                            |

`.env` is loaded from the working directory; existing process
environment takes precedence.

On first load the console offers **Create account**; open it in a browser
at `http://<server>:8080/`, sign in with an account (or the `ADMIN_TOKEN`
via the "server owner" link) and register devices.

## API

- `GET /health` — unauthenticated liveness check.
- `GET /api/sync` — device-facing (`Bearer <device_token>`), ETag/304
  conditional pull (legacy; kept for older firmware).
- `POST /api/sync` — device-facing (`Bearer <device_token>`): uploads
  `enabled`/`done`/importance flags, merges them (unknown IDs ignored),
  returns the authoritative `{alarms, todos}` list. This is what current
  firmware calls — contract in the firmware repo's
  [`docs/sync-api.md`](https://github.com/counhopig/inkpaper-firmware/blob/main/docs/sync-api.md).
- `POST/GET/DELETE /api/devices[/:id]` — admin. `Bearer <ADMIN_TOKEN>` sees
  every device; a console-account session sees only its own devices.
  Registration returns the device token exactly once.
- `/api/devices/:id/alarms` and `/todos` (GET/POST/PUT/DELETE, plus a
  DELETE-to-clear on each collection) — same auth rules as devices.
- `/api/auth/register|login|logout` — console accounts. Register/login
  return a session bearer token (stored client-side); logout revokes it.
- `GET /api/auth/me` — validates a stored session token (or the admin
  token) on console load.
- `POST /api/auth/password` — change your account password (session auth).
- `GET /api/admin/accounts` — owner only (`ADMIN_TOKEN`): list every
  account with its device/session counts.
- `DELETE /api/admin/accounts/:id` — owner only: delete an account (its
  devices and sessions cascade).
- `POST /api/admin/accounts/:id/password` — owner only: reset an account's
  password.

Invalid times, dates, empty names/todos, overlong text and malformed
account credentials are rejected with HTTP 400 before reaching SQLite.

## Console

The embedded UI is a small Vue 3 app split into layered views (no router):

- **Dashboard** — stat strip (devices/alarms/todos/done), device cards,
  register-device.
- **Device** — alarms/todos editor for one device (add, toggle enabled /
  done, priority, delete, clear).
- **Account** — session info, change password; when signed in with the
  `ADMIN_TOKEN`, an admin **Users** panel to list / delete accounts and
  reset their passwords.

## Tests

```bash
cargo test          # 2 unit tests (DB schema + device-state merge)
cd admin-ui && npm run build   # vue-tsc type check + vite build
```

Exercised end-to-end against the physical device via `inkpaper-desktop`.

## License

[Apache-2.0](LICENSE).
