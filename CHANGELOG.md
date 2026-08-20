# Changelog

All notable changes to **inkpaper-server** are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0] - 2026-08-20

### Added
- **Console accounts** — register, log in, log out and change password
  (`POST /api/auth/*`), with Argon2id-hashed passwords and per-account
  device ownership. Account sessions are scoped to the account's own
  devices; admin-token-created devices stay owned by the server owner.
- **Admin Users panel** — owner-only management of console accounts
  (`GET/DELETE /api/admin/accounts`, password reset), listing each
  account with its device/session counts.
- **PostgreSQL support** — storage now runs on sqlx `Any`; set
  `DATABASE_URL` to a `postgres://` URL to use PostgreSQL alongside the
  default zero-config SQLite backend. DDL is emitted per dialect.
- **Hierarchical console views** — login/register screen, and a
  dashboard/device/account view split driven by session state.

### Changed
- Admin console is now a layered Vue 3 view-based app (no router): a new
  `DashboardView`/`DeviceView`/`AccountView`/`LoginView` split replaces
  the previous single-view console.
- Every mutation flows through a shared busy/401 handler; stale responses
  are guarded by a sequence guard so they can't overwrite newer data.
- The release workflow now **publishes** Linux/macOS binaries
  automatically on a `v*` tag instead of leaving a draft.

### Fixed
- Frontend `tsconfig` `moduleResolution` corrected from `Node` to
  `Bundler` so the admin-ui builds correctly.

## [0.1.0] - 2026-08-19

### Added
- Initial backend server: Rust + `axum`, SQLite storage.
- Device registration with one-time per-device bearer tokens; `uuid`
  device keys.
- Alarm and todo CRUD with bulk clear, validation hardening, and
  importance / due-date support.
- Recurring (repeat) schedules for alarms and todos.
- Device-facing `GET` (ETag/304) and bidirectional `POST /api/sync`
  endpoints; `POST` merges only known IDs' `enabled`/`done` flags and
  never lets device data create content.
- Embedded admin console (Vue 3 + Vite) and `start.sh` first-run setup.
- Apache-2.0 license and open-source README.
