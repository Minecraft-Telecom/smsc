use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};

use rusmpp::pdus::SubmitSm;
use rusmpp::types::COctetString;
use tracing::info;

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
}

#[derive(Debug)]
pub struct StubQueue {
    next_id: AtomicU64,
}

impl StubQueue {
    pub fn new() -> Self {
        Self {
            next_id: AtomicU64::new(1),
        }
    }
}

impl MessageQueue for StubQueue {
    fn enqueue(&self, submit: &SubmitSm) -> Result<COctetString<1, 65>, QueueError> {
        let message_id_num = self.next_id.fetch_add(1, Ordering::Relaxed);
        let message_id = format!("msg-{message_id_num}");

        info!(
            source = submit.source_addr.as_str(),
            destination = submit.destination_addr.as_str(),
            sm_length = submit.sm_length(),
            "submit_sm accepted (stub queue)"
        );

        COctetString::from_str(&message_id).map_err(|_| QueueError::MessageId)
    }
}
