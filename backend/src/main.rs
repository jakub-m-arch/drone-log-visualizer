use dji_log_viewer::config::Config;
use dji_log_viewer::{AppState, app, db};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,tower_http=info,sqlx=warn")),
        )
        .init();

    let config = match Config::from_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("configuration error: {e}");
            std::process::exit(2);
        }
    };
    if config.api_key.is_none() {
        tracing::warn!(
            "DJI_API_KEY is not set: logs older than format v13 work, encrypted v13+ logs will be rejected"
        );
    }

    let pool = match db::connect(&config.data_dir).await {
        Ok(p) => p,
        Err(e) => {
            eprintln!("cannot open database in {}: {e}", config.data_dir.display());
            std::process::exit(1);
        }
    };

    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], config.port));
    tracing::info!(
        %addr,
        data_dir = %config.data_dir.display(),
        static_dir = %config.static_dir.display(),
        api_key_configured = config.api_key.is_some(),
        "starting DJI flight log viewer"
    );

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .unwrap_or_else(|e| panic!("cannot bind {addr}: {e}"));
    axum::serve(listener, app(AppState::new(pool, config)))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("server error");
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut s) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            s.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
