mod db;
mod models;
mod routes;

use std::net::SocketAddr;

use anyhow::Context;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "inkpaper_server=info,tower_http=info".into()),
        )
        .init();

    let db_path = std::env::var("DATABASE_PATH").unwrap_or_else(|_| "inkpaper.sqlite3".to_string());
    let admin_token = std::env::var("ADMIN_TOKEN").context(
        "ADMIN_TOKEN env var is required - this is the bearer token inkpaper-desktop uses \
         for device registration and alarm/todo management; generate one long random string \
         and keep it secret, e.g. `openssl rand -hex 32`",
    )?;
    let bind_addr: SocketAddr = std::env::var("BIND_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:8080".to_string())
        .parse()
        .context("BIND_ADDR must be a valid host:port")?;

    let db = db::open(&db_path)?;
    let state = routes::AppState { db, admin_token };
    let app = routes::router(state)
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive());

    tracing::info!("inkpaper-server listening on {bind_addr}, db={db_path}");
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
