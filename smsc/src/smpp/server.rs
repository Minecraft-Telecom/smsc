use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::StreamExt;
use socket2::{SockRef, TcpKeepalive};
use tokio::net::TcpListener;
use tokio::sync::{Semaphore, watch};
use tokio_stream::wrappers::TcpListenerStream;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::config::Config;
use crate::queue::MessageQueue;
use super::session::run_session;

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
    pub fn new(config: Config, queue: Arc<dyn MessageQueue>) -> Self {
        Self {
            config: Arc::new(config),
            queue,
        }
    }

    pub async fn run(self, mut shutdown: watch::Receiver<bool>) -> Result<(), ServerError> {
        let listener = TcpListener::bind(self.config.smpp.bind_addr).await?;
        let mut incoming = TcpListenerStream::new(listener);
        let limiter = Arc::new(Semaphore::new(self.config.smpp.max_connections));
        let ip_limiter = IpConnectionLimiter::new(self.config.smpp.max_connections_per_ip);
        let cancellation_token = CancellationToken::new();

        info!(
            bind_addr = %self.config.smpp.bind_addr,
            max_connections = self.config.smpp.max_connections,
            max_connections_per_ip = self.config.smpp.max_connections_per_ip,
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

                    let ip_guard = match ip_limiter.try_acquire(peer.ip()) {
                        Some(guard) => guard,
                        None => {
                            warn!(peer = %peer, "per-IP connection limit reached");
                            continue;
                        }
                    };

                    if let Err(err) = stream.set_nodelay(true) {
                        warn!(?err, "failed to set TCP_NODELAY");
                    }
                    if let Err(err) = set_keepalive(&stream) {
                        warn!(?err, "failed to set TCP keepalive");
                    }

                    let config = Arc::clone(&self.config);
                    let queue = Arc::clone(&self.queue);
                    let session_token = cancellation_token.clone();

                    tokio::spawn(async move {
                        let _permit = permit;
                        let _ip_guard = ip_guard;

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

fn set_keepalive(stream: &tokio::net::TcpStream) -> std::io::Result<()> {
    let keepalive = TcpKeepalive::new()
        .with_time(Duration::from_secs(60))
        .with_interval(Duration::from_secs(10));
    SockRef::from(stream).set_tcp_keepalive(&keepalive)
}

#[derive(Debug, Clone)]
struct IpConnectionLimiter {
    max_per_ip: usize,
    counts: Arc<Mutex<HashMap<IpAddr, usize>>>,
}

impl IpConnectionLimiter {
    fn new(max_per_ip: usize) -> Self {
        Self {
            max_per_ip,
            counts: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn try_acquire(&self, ip: IpAddr) -> Option<IpConnectionGuard> {
        let mut counts = self
            .counts
            .lock()
            .expect("IP connection limiter mutex poisoned");
        let count = counts.entry(ip).or_insert(0);
        if *count >= self.max_per_ip {
            return None;
        }
        *count += 1;
        Some(IpConnectionGuard {
            ip,
            counts: Arc::clone(&self.counts),
        })
    }
}

#[derive(Debug)]
struct IpConnectionGuard {
    ip: IpAddr,
    counts: Arc<Mutex<HashMap<IpAddr, usize>>>,
}

impl Drop for IpConnectionGuard {
    fn drop(&mut self) {
        let mut counts = self
            .counts
            .lock()
            .expect("IP connection limiter mutex poisoned");
        if let Some(count) = counts.get_mut(&self.ip) {
            *count -= 1;
            if *count == 0 {
                counts.remove(&self.ip);
            }
        }
    }
}
