use std::sync::Arc;

use futures::StreamExt;
use tokio::net::TcpListener;
use tokio::sync::{watch, Semaphore};
use tokio_stream::wrappers::TcpListenerStream;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::config::Config;
use crate::queue::{MessageQueue, StubQueue};
use crate::session::run_session;

#[derive(Debug)]
pub enum ServerError {
    Io(std::io::Error),
}

impl std::fmt::Display for ServerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServerError::Io(err) => write!(f, "io error: {err}"),
        }
    }
}

impl std::error::Error for ServerError {}

impl From<std::io::Error> for ServerError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

pub struct Server {
    config: Arc<Config>,
    queue: Arc<dyn MessageQueue>,
}

impl Server {
    pub fn new(config: Config) -> Self {
        let queue: Arc<dyn MessageQueue> = Arc::new(StubQueue::new(config.queue_broadcast_capacity));

        Self {
            config: Arc::new(config),
            queue,
        }
    }

    pub async fn run(self, mut shutdown: watch::Receiver<bool>) -> Result<(), ServerError> {
        let listener = TcpListener::bind(self.config.bind_addr).await?;
        let mut incoming = TcpListenerStream::new(listener);
        let limiter = Arc::new(Semaphore::new(self.config.max_connections));
        let cancellation_token = CancellationToken::new();

        info!(
            bind_addr = %self.config.bind_addr,
            max_connections = self.config.max_connections,
            "SMSC listening"
        );

        loop {
            tokio::select! {
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        info!("shutdown signal received");
                        cancellation_token.cancel();
                        break;
                    }
                }
                maybe_stream = incoming.next() => {
                    let stream = match maybe_stream {
                        Some(Ok(stream)) => stream,
                        Some(Err(err)) => {
                            warn!(?err, "failed to accept connection");
                            continue;
                        }
                        None => break,
                    };

                    let peer = match stream.peer_addr() {
                        Ok(addr) => addr,
                        Err(err) => {
                            warn!(?err, "failed to read peer address");
                            continue;
                        }
                    };

                    let permit = tokio::select! {
                        _ = shutdown.changed() => {
                            info!("shutdown during permit wait");
                            cancellation_token.cancel();
                            break;
                        }
                        res = limiter.clone().acquire_owned() => match res {
                            Ok(p) => p,
                            Err(_) => { warn!("connection limiter closed"); break; }
                        }
                    };

                    if let Err(err) = stream.set_nodelay(true) {
                        warn!(?err, "failed to set TCP_NODELAY");
                    }

                    let config = Arc::clone(&self.config);
                    let queue = Arc::clone(&self.queue);
                    let session_token = cancellation_token.clone();

                    tokio::spawn(async move {
                        let _permit = permit;

                        if let Err(err) = run_session(stream, peer, config, queue, session_token).await {
                            warn!(?err, "session terminated with error");
                        }
                    });
                }
            }
        }

        info!("server stopped");
        Ok(())
    }
}
