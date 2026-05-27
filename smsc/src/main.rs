mod config;
mod queue;
mod server;
mod session;

use config::Config;
use server::Server;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let config = Config::from_file()?;

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(config.log_filter.as_str()));

    tracing_subscriber::fmt().with_env_filter(filter).init();

    let server = Server::new(config);
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let server_task = tokio::spawn(server.run(shutdown_rx));

    wait_for_shutdown_signal().await?;
    info!("shutdown requested");
    _ = shutdown_tx.send(true);

    match server_task.await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(err)) => Err(err.into()),
        Err(err) => Err(err.into()),
    }
}

#[cfg(unix)]
async fn wait_for_shutdown_signal() -> std::io::Result<()> {
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;

    tokio::select! {
        res = tokio::signal::ctrl_c() => res,
        _ = sigterm.recv() => Ok(()),
    }
}

#[cfg(not(unix))]
async fn wait_for_shutdown_signal() -> std::io::Result<()> {
    tokio::signal::ctrl_c().await
}
