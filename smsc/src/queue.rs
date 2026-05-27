use std::collections::VecDeque;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use dashmap::DashMap;
use rusmpp::pdus::SubmitSm;
use rusmpp::types::COctetString;
use tokio::sync::Notify;
use tracing::debug;

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
    Processing,
    Delivered,
    Failed,
}

impl std::fmt::Display for MessageStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MessageStatus::Queued => write!(f, "queued"),
            MessageStatus::Processing => write!(f, "processing"),
            MessageStatus::Delivered => write!(f, "delivered"),
            MessageStatus::Failed => write!(f, "failed"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct QueueMessage {
    message_id: String,
    pub submit: SubmitSm,
    pub status: MessageStatus,
}

impl QueueMessage {
    pub fn new(message_id: String, submit: SubmitSm, status: MessageStatus) -> Result<Self, QueueError> {
        let _ = COctetString::<1, 65>::from_str(&message_id).map_err(|_| QueueError::MessageId)?;
        Ok(Self { message_id, submit, status })
    }

    pub fn message_id(&self) -> COctetString<1, 65> {
        COctetString::from_str(&self.message_id).expect("validated message id")
    }

    pub fn message_id_str(&self) -> &str {
        &self.message_id
    }
}

pub trait MessageQueue: Send + Sync {
    fn enqueue(&self, submit: &SubmitSm) -> Result<QueueMessage, QueueError>;
    fn dequeue(&self) -> std::pin::Pin<Box<dyn std::future::Future<Output = QueueMessage> + Send + '_>>;
    fn status(&self, message_id: &str) -> Option<MessageStatus>;
    fn update_status(&self, message_id: &str, status: MessageStatus);
}

#[derive(Debug)]
pub struct InMemoryQueue {
    next_id: AtomicU64,
    records: DashMap<String, QueueMessage>,
    pending: Mutex<VecDeque<String>>,
    notify: Notify,
}

impl InMemoryQueue {
    pub fn new(_capacity: usize) -> Self {
        Self {
            next_id: AtomicU64::new(1),
            records: DashMap::new(),
            pending: Mutex::new(VecDeque::new()),
            notify: Notify::new(),
        }
    }
}

impl Default for InMemoryQueue {
    fn default() -> Self {
        Self::new(1024)
    }
}

impl MessageQueue for InMemoryQueue {
    fn enqueue(&self, submit: &SubmitSm) -> Result<QueueMessage, QueueError> {
        let message_id_num = self.next_id.fetch_add(1, Ordering::Relaxed);
        let message_id_raw = format!("msg-{}", message_id_num);
        
        let msg = QueueMessage::new(message_id_raw.clone(), submit.clone(), MessageStatus::Queued)?;

        self.records.insert(message_id_raw.clone(), msg.clone());
        
        {
            let mut pending = self.pending.lock().unwrap();
            pending.push_back(message_id_raw);
        }

        self.notify.notify_waiters();

        debug!(
            source = submit.source_addr.as_str(),
            destination = submit.destination_addr.as_str(),
            sm_length = submit.sm_length(),
            message_id = msg.message_id_str(),
            "submit_sm accepted"
        );

        Ok(msg)
    }

    fn dequeue(&self) -> std::pin::Pin<Box<dyn std::future::Future<Output = QueueMessage> + Send + '_>> {
        Box::pin(async {
            loop {
                let notified = self.notify.notified();
                
                let popped = {
                    let mut pending = self.pending.lock().unwrap();
                    if let Some(id_raw) = pending.pop_front() {
                        if let Some(mut record) = self.records.get_mut(&id_raw) {
                            record.status = MessageStatus::Processing;
                            Some(record.clone())
                        } else { None }
                    } else { None }
                };

                if let Some(msg) = popped {
                    return msg;
                }

                notified.await;
            }
        })
    }

    fn status(&self, message_id: &str) -> Option<MessageStatus> {
        self.records.get(message_id).map(|r| r.status)
    }

    fn update_status(&self, message_id: &str, status: MessageStatus) {
        if let Some(mut record) = self.records.get_mut(message_id) {
            record.status = status;
            debug!(message_id, ?status, "message status updated");
        }
    }
}

