use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};

use rusmpp::pdus::SubmitSm;
use rusmpp::types::COctetString;
use tokio::sync::broadcast;
use tracing::{debug, info, warn};

#[derive(Debug)]
pub enum QueueError {
    MessageId,
}

impl std::fmt::Display for QueueError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QueueError::MessageId => write!(f, "failed to generate message id"),
        }
    }
}

impl std::error::Error for QueueError {}

pub trait MessageQueue: Send + Sync {
    fn enqueue(&self, submit: &SubmitSm) -> Result<COctetString<1, 65>, QueueError>;
    fn subscribe(&self) -> broadcast::Receiver<QueueMessage>;
}

#[derive(Debug, Clone)]
pub struct QueueMessage {
    pub message_id: COctetString<1, 65>,
    pub submit: SubmitSm,
}

#[derive(Debug)]
pub struct StubQueue {
    next_id: AtomicU64,
    tx: broadcast::Sender<QueueMessage>,
}

impl StubQueue {
    pub fn new(capacity: usize) -> Self {
        let (tx, _rx) = broadcast::channel(capacity);
        Self {
            next_id: AtomicU64::new(1),
            tx,
        }
    }
}

impl Default for StubQueue {
    fn default() -> Self {
        Self::new(1024)
    }
}

impl MessageQueue for StubQueue {
    fn enqueue(&self, submit: &SubmitSm) -> Result<COctetString<1, 65>, QueueError> {
        let message_id_num = self.next_id.fetch_add(1, Ordering::Relaxed);
        let message_id_raw = format!("msg-{message_id_num}");

        let message_id =
            COctetString::from_str(&message_id_raw).map_err(|_| QueueError::MessageId)?;

        let queued = QueueMessage {
            message_id: message_id.clone(),
            submit: submit.clone(),
        };

        match self.tx.send(queued) {
            Ok(subscribers) => {
                debug!(subscribers, "submit_sm broadcast to clients");
            }
            Err(_) => {
                warn!("submit_sm broadcast dropped; no active receivers");
            }
        }

        info!(
            source = submit.source_addr.as_str(),
            destination = submit.destination_addr.as_str(),
            sm_length = submit.sm_length(),
            message_id = message_id.as_str(),
            "submit_sm accepted (stub queue)"
        );

        Ok(message_id)
    }

    fn subscribe(&self) -> broadcast::Receiver<QueueMessage> {
        self.tx.subscribe()
    }
}
