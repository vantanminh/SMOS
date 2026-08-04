//! SMOS — Server Management OS entry point.

use anyhow::{Context, Result};
use clap::Parser;
use smos::api::{self, resolve_static_dir};
use smos::config::SmosConfig;
use smos::state::AppState;
use std::net::SocketAddr;
use std::path::PathBuf;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

#[derive(Debug, Parser)]
#[command(
    name = "smos",
    about = "SMOS — Server Management OS: metrics, processes, logs, config, audit, WebUI"
)]
struct Cli {
    /// HTTP bind address (host:port). Overrides config file.
    #[arg(long, env = "SMOS_BIND")]
    bind: Option<String>,

    /// Data directory for config, audit, and service logs.
    #[arg(long, env = "SMOS_DATA_DIR", default_value = "smos-data")]
    data_dir: PathBuf,

    /// Optional shared secret for mutating API calls.
    #[arg(long, env = "SMOS_AUTH_TOKEN")]
    auth_token: Option<String>,

    /// Host label shown in the dashboard.
    #[arg(long, env = "SMOS_HOST_LABEL")]
    host_label: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Ensure data dir exists before logging setup
    std::fs::create_dir_all(&cli.data_dir)
        .with_context(|| format!("create data dir {}", cli.data_dir.display()))?;

    // Daily rotation so log history can be retained / pruned by calendar day.
    // Files: smos.log.YYYY-MM-DD under data_dir (plus active writer file).
    let file_appender = tracing_appender::rolling::daily(&cli.data_dir, "smos.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt::layer().with_writer(std::io::stdout))
        .with(fmt::layer().with_ansi(false).with_writer(non_blocking))
        .init();

    let mut config = SmosConfig::load_or_default(&cli.data_dir)?;
    if let Some(bind) = cli.bind {
        config.bind = bind;
    }
    if let Some(token) = cli.auth_token {
        config.auth_token = Some(token);
    }
    if let Some(label) = cli.host_label {
        config.host_label = label;
    }
    config.validate()?;
    config.save()?;

    // Seed a startup line into the service log for log-API verification.
    let _ = smos::logs::append_service_line(
        &config.data_dir,
        &format!(
            "{} INFO smos starting bind={} host_label={}",
            chrono::Utc::now().to_rfc3339(),
            config.bind,
            config.host_label
        ),
    );

    let bind: SocketAddr = config
        .bind
        .parse()
        .or_else(|_| {
            // hostname:port form
            use std::net::ToSocketAddrs;
            config
                .bind
                .to_socket_addrs()
                .ok()
                .and_then(|mut i| i.next())
                .ok_or_else(|| anyhow::anyhow!("cannot resolve bind {}", config.bind))
        })
        .with_context(|| format!("parse bind address {}", config.bind))?;

    let static_dir = resolve_static_dir();
    tracing::info!(
        bind = %bind,
        data_dir = %config.data_dir.display(),
        static_dir = %static_dir.display(),
        host_label = %config.host_label,
        "SMOS starting"
    );

    let state = AppState::new(config);
    // Background: sample metrics history + prune past retention.
    tokio::spawn(history_worker(state.clone()));

    let app = api::router(state, static_dir);

    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .with_context(|| format!("bind {bind}"))?;

    tracing::info!("SMOS listening on http://{bind}  (WebUI /  API /api/health)");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("server error")?;

    Ok(())
}

/// Persist metrics samples and prune metrics/logs older than retention.
async fn history_worker(state: AppState) {
    // Initial prune shortly after start.
    {
        let cfg = state.inner.config.read().clone();
        let data_dir = cfg.data_dir.clone();
        let days = cfg.history_retention_days;
        let _ = tokio::task::spawn_blocking(move || smos::history::prune_all(&data_dir, days)).await;
    }

    let mut last_prune = std::time::Instant::now();
    loop {
        let (interval_secs, data_dir, retention_days) = {
            let cfg = state.inner.config.read();
            (
                cfg.metrics_history_interval_secs.max(10),
                cfg.data_dir.clone(),
                cfg.history_retention_days,
            )
        };

        tokio::time::sleep(std::time::Duration::from_secs(interval_secs)).await;

        let dir = data_dir.clone();
        match tokio::task::spawn_blocking(move || {
            let snap = smos::metrics::collect_metrics();
            smos::history::record_metrics_snapshot(&dir, &snap)
        })
        .await
        {
            Ok(Ok(())) => {}
            Ok(Err(e)) => tracing::warn!("metrics history sample failed: {e}"),
            Err(e) => tracing::warn!("metrics history task join failed: {e}"),
        }

        // Prune at most hourly (or when interval is longer).
        let prune_every = std::time::Duration::from_secs(3600);
        if last_prune.elapsed() >= prune_every {
            let dir = data_dir.clone();
            let days = retention_days;
            match tokio::task::spawn_blocking(move || smos::history::prune_all(&dir, days)).await {
                Ok(Ok((m, l))) if m > 0 || l > 0 => {
                    tracing::info!(metrics_pruned = m, logs_pruned = l, "history prune complete");
                }
                Ok(Ok(_)) => {}
                Ok(Err(e)) => tracing::warn!("history prune failed: {e}"),
                Err(e) => tracing::warn!("history prune task join failed: {e}"),
            }
            last_prune = std::time::Instant::now();
        }
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    tracing::info!("shutdown signal received");
}
