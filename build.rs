// Builds the embedded admin console (admin-ui/, a Vue 3 + Vite app) before
// compiling, so `cargo build`/`cargo run` alone produces a working binary
// with the current UI baked in - see routes.rs's `rust_embed` usage, which
// reads admin-ui/dist/ at compile time. Requires a one-time `npm install`
// in admin-ui/ (Node.js/npm must be installed); see admin-ui/README.md.

use std::path::Path;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=admin-ui/src");
    println!("cargo:rerun-if-changed=admin-ui/index.html");
    println!("cargo:rerun-if-changed=admin-ui/package.json");
    println!("cargo:rerun-if-changed=admin-ui/vite.config.ts");

    if !Path::new("admin-ui/node_modules").exists() {
        panic!(
            "admin-ui/node_modules is missing - run `npm install` in admin-ui/ once before building \
             (requires Node.js/npm)"
        );
    }

    // The ts-rs bindings regeneration path (`npm run codegen` in admin-ui/)
    // runs `cargo test` while the committed generated/*.ts may be stale or
    // broken by in-progress DTO edits - exactly when vue-tsc would fail and
    // panic this build script before the export tests can run. Skip the UI
    // build there; every other cargo invocation still builds the UI.
    if std::env::var_os("INKWASH_SKIP_UI_BUILD").is_some() {
        return;
    }

    let status = Command::new("npm")
        .args(["run", "build"])
        .current_dir("admin-ui")
        .status();

    match status {
        Ok(s) if s.success() => {}
        Ok(s) => panic!("`npm run build` in admin-ui/ failed with {s}"),
        Err(e) => panic!("failed to run `npm run build` in admin-ui/: {e} (is npm on PATH?)"),
    }
}
