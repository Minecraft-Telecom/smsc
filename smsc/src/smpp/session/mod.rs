mod bind;
mod deliver;
mod delivery;
mod handler;
mod io;
mod run;

use rusmpp::tokio_codec::{DecodeError, EncodeError};

#[derive(Debug)]
pub enum SessionError {
    Decode(DecodeError),
    Encode(EncodeError),
    ReceiptOverflow,
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionError::Decode(err) => write!(f, "decode error: {err}"),
            SessionError::Encode(err) => write!(f, "encode error: {err}"),
            SessionError::ReceiptOverflow => write!(f, "delivery receipt text overflow"),
        }
    }
}

impl std::error::Error for SessionError {}

impl From<DecodeError> for SessionError {
    fn from(err: DecodeError) -> Self {
        Self::Decode(err)
    }
}

impl From<EncodeError> for SessionError {
    fn from(err: EncodeError) -> Self {
        Self::Encode(err)
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(crate) enum BindState {
    Unbound,
    Transmitter,
    Receiver,
    Transceiver,
}

impl BindState {
    fn allows_tx(self) -> bool {
        matches!(self, BindState::Transmitter | BindState::Transceiver)
    }

    fn allows_rx(self) -> bool {
        matches!(self, BindState::Receiver | BindState::Transceiver)
    }
}

#[derive(Debug, Copy, Clone)]
enum BindKind {
    Transmitter,
    Receiver,
    Transceiver,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum SessionAction {
    Continue,
    Close,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum BindOutcome {
    Accepted,
    Rejected,
    AlreadyBound,
}

pub(crate) struct SessionData {
    pub state: BindState,
    pub next_sequence: u32,
    pub bind_failures: usize,
    pub pending_deliveries: deliver::PendingDeliveries,
}

pub use run::run_session;

fn next_sequence_number(counter: &mut u32) -> u32 {
    let sequence = *counter;
    *counter = counter.wrapping_add(1);
    if *counter == 0 {
        *counter = 1;
    }
    sequence
}
