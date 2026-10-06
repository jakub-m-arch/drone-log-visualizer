use drone_log_visualizer::config::Config;
use drone_log_visualizer::{AppState, app, db};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    // `drone-log-visualizer healthcheck` is used by the Docker HEALTHCHECK (the
    // runtime image has no curl).
    if std::env::args().nth(1).as_deref() == Some("healthcheck") {
        std::process::exit(healthcheck().await);
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,tower_http=info,sqlx=warn")),
        )
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stdout()))
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
        "starting Drone Log Visualizer"
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

async fn healthcheck() -> i32 {
    let port = std::env::var("PORT").unwrap_or_else(|_| "8080".into());
    let client = match reqwest::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(3))
        .build()
    {
        Ok(c) => c,
        Err(_) => return 1,
    };
    match client
        .get(format!("http://127.0.0.1:{port}/api/config"))
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => 0,
        _ => 1,
    }
}
