pub mod api;
pub mod config;
pub mod db;
pub mod error;
pub mod export;
pub mod flight;
pub mod ingest;
pub mod keychain;
pub mod synthetic;

use axum::Router;
use sqlx::SqlitePool;
use std::sync::Arc;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;

use config::Config;

/// Version of `dji-log-parser` this build is pinned to (see Cargo.toml).
pub const PARSER_VERSION: &str = "0.5.7";

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub config: Arc<Config>,
    pub http: reqwest::Client,
}

impl AppState {
    pub fn new(pool: SqlitePool, config: Config) -> Self {
        AppState {
            pool,
            config: Arc::new(config),
            http: keychain::http_client(),
        }
    }
}

/// Full application: JSON API plus the static frontend (SPA fallback to index.html).
pub fn app(state: AppState) -> Router {
    let static_dir = state.config.static_dir.clone();
    let spa = ServeDir::new(&static_dir).fallback(ServeFile::new(static_dir.join("index.html")));
    api::router(state)
        .fallback_service(spa)
        .layer(TraceLayer::new_for_http())
}
