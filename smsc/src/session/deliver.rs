use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Duration;

use rusmpp::tokio_codec::CommandCodec;
use rusmpp::{Command, CommandId, CommandStatus};
use tokio::net::TcpStream;
use tokio::time::Instant;
use tokio_util::codec::Framed;
use tracing::{debug, warn};

use super::SessionError;
use super::io::send_command;

#[derive(Debug)]
pub(super) struct PendingDeliveries {
    timeout: Duration,
    entries: HashMap<u32, PendingDelivery>,
}

#[derive(Debug)]
struct PendingDelivery {
    sent_at: Instant,
    source: DeliverySource,
}

#[derive(Debug)]
pub(super) enum DeliverySource {
    Queue { message_id: String },
    Receipt { message_id: String },
}

impl DeliverySource {
    fn kind(&self) -> &'static str {
        match self {
            DeliverySource::Queue { .. } => "queue",
            DeliverySource::Receipt { .. } => "receipt",
        }
    }

    fn message_id(&self) -> &str {
        match self {
            DeliverySource::Queue { message_id } | DeliverySource::Receipt { message_id } => {
                message_id
            }
        }
    }
}

impl PendingDeliveries {
    pub(super) fn new(timeout: Duration) -> Self {
        Self {
            timeout,
            entries: HashMap::new(),
        }
    }

    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(super) async fn send(
        &mut self,
        framed: &mut Framed<TcpStream, CommandCodec>,
        peer: SocketAddr,
        command: Command,
        source: DeliverySource,
    ) -> Result<(), SessionError> {
        let sequence = command.sequence_number();
        send_command(framed, peer, command).await?;
        self.entries.insert(
            sequence,
            PendingDelivery {
                sent_at: Instant::now(),
                source,
            },
        );
        Ok(())
    }

    pub(super) fn handle_response(&mut self, peer: SocketAddr, command: &Command) -> bool {
        if command.id() != CommandId::DeliverSmResp {
            return false;
        }

        let sequence = command.sequence_number();
        match self.entries.remove(&sequence) {
            Some(pending) => {
                if command.status() == CommandStatus::EsmeRok {
                    debug!(
                        peer = %peer,
                        sequence,
                        source = pending.source.kind(),
                        message_id = pending.source.message_id(),
                        "deliver_sm acknowledged"
                    );
                } else {
                    warn!(
                        peer = %peer,
                        sequence,
                        status = ?command.status(),
                        source = pending.source.kind(),
                        message_id = pending.source.message_id(),
                        "deliver_sm rejected by peer"
                    );
                }
            }
            None => {
                warn!(peer = %peer, sequence, "unexpected deliver_sm_resp");
            }
        }

        true
    }

    pub(super) fn expire(&mut self, peer: SocketAddr) {
        let now = Instant::now();
        self.entries.retain(|sequence, pending| {
            let keep = now.duration_since(pending.sent_at) < self.timeout;
            if !keep {
                warn!(
                    peer = %peer,
                    sequence,
                    source = pending.source.kind(),
                    message_id = pending.source.message_id(),
                    "deliver_sm response timed out"
                );
            }
            keep
        });
    }
}
