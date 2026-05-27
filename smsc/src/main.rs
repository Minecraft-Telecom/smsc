mod config;
mod http;
mod queue;
mod smpp;

use std::sync::Arc;
use config::Config;
use queue::{InMemoryQueue, MessageQueue};
use smpp::server::Server;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let config = Config::from_file()?;

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(config.log_filter.as_str()));

    tracing_subscriber::fmt().with_env_filter(filter).init();

    let queue: Arc<dyn MessageQueue> =
        Arc::new(InMemoryQueue::new(config.smpp.queue_broadcast_capacity));

    let server = Server::new(config.clone(), queue.clone());
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    
    let smpp_task = tokio::spawn(server.run(shutdown_rx));
    let http_task = tokio::spawn(http::run_http_server(config.http.bind_addr, queue));

    wait_for_shutdown_signal().await?;
    info!("shutdown requested");
    _ = shutdown_tx.send(true);

    let _ = tokio::join!(smpp_task, http_task);

    Ok(())
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
