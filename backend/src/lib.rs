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
        // Fingerprinted bundles: a missing file is a 404, not the SPA page.
        .nest_service("/assets", ServeDir::new(static_dir.join("assets")))
        .fallback_service(spa)
        .layer(axum::middleware::map_response(cache_headers))
        .layer(TraceLayer::new_for_http())
}

/// Vite fingerprints everything under /assets/, so those files can be cached
/// forever; everything else (index.html, the API) must be revalidated so a
/// rebuilt image is picked up without a hard refresh.
async fn cache_headers(
    uri: axum::http::Uri,
    mut res: axum::response::Response,
) -> axum::response::Response {
    use axum::http::{HeaderValue, header};
    let value = if uri.path().starts_with("/assets/") && res.status().is_success() {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    res.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static(value));
    res
}
