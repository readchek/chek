//! The Chek library server.

mod api;
mod auth;
mod config;
mod limit;
mod opds;
mod progress;
mod telemetry;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::serve::ListenerExt;
use chek_catalog::Catalog;
use chek_library::ScanOptions;
use tracing::{error, info, warn};

use crate::config::Config;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let telemetry = telemetry::init()?;
    let result = run().await;
    if let Err(e) = &result {
        error!(error = %e, "server stopped");
    }
    telemetry.shutdown();
    result
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::from_env()?;
    std::fs::create_dir_all(&config.data_dir)?;
    let catalog = Arc::new(Catalog::open(&config.data_dir.join("chek.db"))?);
    let interrupted = catalog.reset_interrupted_scans()?;
    if interrupted > 0 {
        warn!(
            interrupted,
            "marked scans interrupted by the last shutdown as failed"
        );
    }

    let state = api::AppState {
        catalog,
        scan: ScanOptions {
            workers: config.scan_workers,
            covers_dir: config.data_dir.join("covers"),
        },
        setup_token: Arc::default(),
        basic_auth_cache: Arc::default(),
        password_limiter: Arc::default(),
        trusted_proxies: Arc::new(config.trusted_proxies),
    };
    auth::prepare_setup(&state, config.setup_token).await?;
    // Unchanged files are skipped by size and modification time, so this is
    // cheap, and it catches whatever changed while the server was down.
    api::scan_all_folders(&state).await?;
    let app = api::router(state);

    // Without TCP_NODELAY, small responses wait on Nagle's algorithm and
    // delayed ACKs; the latency spike measured p99 at 84 ms instead of 50.
    let listener = tokio::net::TcpListener::bind(config.bind)
        .await?
        .tap_io(|tcp| {
            if let Err(e) = tcp.set_nodelay(true) {
                warn!(error = %e, "could not set TCP_NODELAY");
            }
        });
    info!(address = %config.bind, data_dir = %config.data_dir.display(), "listening");
    // Connection addresses let sign-in limit password guesses per client.
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(_) => std::future::pending().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
}
