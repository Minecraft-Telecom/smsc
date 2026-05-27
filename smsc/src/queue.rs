use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};

use dashmap::DashMap;
use rusmpp::pdus::SubmitSm;
use rusmpp::types::COctetString;
use tokio::sync::broadcast;
use tracing::{debug, warn};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageStatus {
    Queued,
    Delivered,
    Failed,
}

impl std::fmt::Display for MessageStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MessageStatus::Queued => write!(f, "queued"),
            MessageStatus::Delivered => write!(f, "delivered"),
            MessageStatus::Failed => write!(f, "failed"),
        }
    }
}

pub trait MessageQueue: Send + Sync {
    fn enqueue(&self, submit: &SubmitSm) -> Result<COctetString<1, 65>, QueueError>;
    fn subscribe(&self) -> broadcast::Receiver<QueueMessage>;
    fn status(&self, message_id: &str) -> Option<MessageStatus>;
    fn update_status(&self, message_id: &str, status: MessageStatus);
}

#[derive(Debug, Clone)]
pub struct QueueMessage {
    pub message_id: COctetString<1, 65>,
    pub submit: SubmitSm,
}

#[derive(Debug)]
pub struct InMemoryQueue {
    next_id: AtomicU64,
    tx: broadcast::Sender<QueueMessage>,
    statuses: DashMap<String, MessageStatus>,
}

impl InMemoryQueue {
    pub fn new(capacity: usize) -> Self {
        let (tx, _rx) = broadcast::channel(capacity);
        Self {
            next_id: AtomicU64::new(1),
            tx,
            statuses: DashMap::new(),
        }
    }
}

impl Default for InMemoryQueue {
    fn default() -> Self {
        Self::new(1024)
    }
}

impl MessageQueue for InMemoryQueue {
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

        debug!(
            source = submit.source_addr.as_str(),
            destination = submit.destination_addr.as_str(),
            sm_length = submit.sm_length(),
            message_id = message_id.as_str(),
            "submit_sm accepted"
        );

        self.statuses.insert(message_id_raw, MessageStatus::Queued);

        Ok(message_id)
    }

    fn subscribe(&self) -> broadcast::Receiver<QueueMessage> {
        self.tx.subscribe()
    }

    fn status(&self, message_id: &str) -> Option<MessageStatus> {
        self.statuses.get(message_id).map(|entry| *entry.value())
    }

    fn update_status(&self, message_id: &str, status: MessageStatus) {
        if let Some(mut entry) = self.statuses.get_mut(message_id) {
            *entry.value_mut() = status;
            debug!(message_id, ?status, "message status updated");
        }
    }
}

