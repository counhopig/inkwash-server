# Inkwash Console

The admin frontend for [`inkwash-server`](..) - registers devices and
manages their alarms/todos. Vue 3 + Vite + TypeScript, no other
dependencies (no router, no state library; it's a single page).

Always served same-origin by `inkwash-server` at `/` (see `../src/routes.rs`,
which embeds this app's `dist/` output into the server binary at compile
time via `build.rs`), so every API call is a relative `fetch()` - there's
no configurable server URL, only the admin bearer token.

## Dev workflow

```bash
npm install
npm run dev       # served at :5173, proxies /api and /health to :8080
```

Run `inkwash-server` separately (`cargo run`, default port 8080) so the
dev server has something to proxy to.

```bash
npm run build      # type-checks (vue-tsc) then builds dist/
```

You normally don't need to run `build` by hand - `inkwash-server`'s
`build.rs` does it automatically on `cargo build`/`cargo run`.

## Structure

- `src/App.vue` - the whole UI: server access, device list/registration,
  alarms, todos.
- `src/lib/api.ts` - typed `fetch()` wrappers over the admin API.
- `src/lib/clipboard.ts` - copy-to-clipboard with a fallback for insecure
  contexts (`navigator.clipboard` is unavailable over plain `http://` on
  a non-localhost address, which is how this console is normally opened
  on the LAN).
- `src/lib/storage.ts` - persists the admin token in `localStorage`.
- `src/style.css` - monochrome/monospace styling matching the physical
  device's own e-ink look.
