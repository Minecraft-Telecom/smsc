use std::net::SocketAddr;
use std::sync::Arc;

use futures::{SinkExt, StreamExt};
use rusmpp::pdus::{BindReceiverResp, BindTransceiverResp, BindTransmitterResp, SubmitSmResp};
use rusmpp::tokio_codec::{CommandCodec, DecodeError, EncodeError};
use rusmpp::types::COctetString;
use rusmpp::values::InterfaceVersion;
use rusmpp::{Command, CommandStatus, Pdu};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_util::codec::Framed;
use tracing::{debug, info, warn};

use crate::config::Config;
use crate::queue::MessageQueue;

#[derive(Debug)]
pub enum SessionError {
    Decode(DecodeError),
    Encode(EncodeError),
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionError::Decode(err) => write!(f, "decode error: {err}"),
            SessionError::Encode(err) => write!(f, "encode error: {err}"),
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
enum BindState {
    Unbound,
    Transmitter,
    Receiver,
    Transceiver,
}

impl BindState {
    fn allows_tx(self) -> bool {
        matches!(self, BindState::Transmitter | BindState::Transceiver)
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

pub async fn run_session(
    stream: TcpStream,
    peer: SocketAddr,
    config: Arc<Config>,
    queue: Arc<dyn MessageQueue>,
) -> Result<(), SessionError> {
    let codec = CommandCodec::new().with_max_length(config.max_pdu_length);
    let mut framed = Framed::new(stream, codec);
    let mut state = BindState::Unbound;

    info!(peer = %peer, "SMPP session started");

    loop {
        let next = timeout(config.idle_timeout, framed.next()).await;
        let command = match next {
            Ok(Some(Ok(command))) => command,
            Ok(Some(Err(err))) => return Err(SessionError::Decode(err)),
            Ok(None) => {
                info!(peer = %peer, "peer disconnected");
                return Ok(());
            }
            Err(_) => {
                info!(peer = %peer, "idle timeout, closing session");
                return Ok(());
            }
        };

        info!(peer = %peer, command = ?command, "rx");

        if command.id().is_response() {
            debug!(peer = %peer, command_id = ?command.id(), "ignoring response PDU");
            continue;
        }

        let action =
            handle_command(&mut framed, peer, &mut state, &config, &queue, command).await?;
        if action == SessionAction::Close {
            info!(peer = %peer, "session closed by peer");
            break;
        }
    }

    Ok(())
}

async fn handle_command(
    framed: &mut Framed<TcpStream, CommandCodec>,
    peer: SocketAddr,
    state: &mut BindState,
    config: &Config,
    queue: &Arc<dyn MessageQueue>,
    command: Command,
) -> Result<SessionAction, SessionError> {
    let sequence = command.sequence_number();
    let pdu = match command.pdu() {
        Some(pdu) => pdu,
        None => {
            send_nack(framed, peer, sequence, CommandStatus::EsmeRinvcmdid).await?;
            return Ok(SessionAction::Continue);
        }
    };

    match pdu {
        Pdu::BindTransmitter(bind) => {
            handle_bind(
                framed,
                peer,
                state,
                config,
                BindKind::Transmitter,
                bind.system_id.as_str(),
                bind.password.as_str(),
                bind.interface_version,
                sequence,
            )
            .await?;
        }
        Pdu::BindReceiver(bind) => {
            handle_bind(
                framed,
                peer,
                state,
                config,
                BindKind::Receiver,
                bind.system_id.as_str(),
                bind.password.as_str(),
                bind.interface_version,
                sequence,
            )
            .await?;
        }
        Pdu::BindTransceiver(bind) => {
            handle_bind(
                framed,
                peer,
                state,
                config,
                BindKind::Transceiver,
                bind.system_id.as_str(),
                bind.password.as_str(),
                bind.interface_version,
                sequence,
            )
            .await?;
        }
        Pdu::EnquireLink => {
            let response = Command::new(CommandStatus::EsmeRok, sequence, Pdu::EnquireLinkResp);
            send_command(framed, peer, response).await?;
        }
        Pdu::SubmitSm(submit) => {
            if !state.allows_tx() {
                send_nack(framed, peer, sequence, CommandStatus::EsmeRinvbndsts).await?;
                return Ok(SessionAction::Continue);
            }

            let message_id = match queue.enqueue(&submit) {
                Ok(message_id) => message_id,
                Err(_) => {
                    warn!("submit_sm rejected by queue stub");
                    COctetString::<1, 65>::empty()
                }
            };

            let status = if message_id.is_empty() {
                CommandStatus::EsmeRsubmitfail
            } else {
                CommandStatus::EsmeRok
            };

            let resp = SubmitSmResp::builder().message_id(message_id).build();
            let response = Command::new(status, sequence, resp);
            send_command(framed, peer, response).await?;
        }
        Pdu::Unbind => {
            let response = Command::new(CommandStatus::EsmeRok, sequence, Pdu::UnbindResp);
            send_command(framed, peer, response).await?;
            return Ok(SessionAction::Close);
        }
        Pdu::GenericNack => {
            debug!("received generic_nack");
        }
        Pdu::UnbindResp
        | Pdu::EnquireLinkResp
        | Pdu::BindReceiverResp(_)
        | Pdu::BindTransceiverResp(_)
        | Pdu::BindTransmitterResp(_)
        | Pdu::SubmitSmResp(_)
        | Pdu::CancelSmResp
        | Pdu::ReplaceSmResp
        | Pdu::CancelBroadcastSmResp => {
            debug!(command_id = ?command.id(), "ignoring unexpected response PDU");
        }
        _ => {
            send_nack(framed, peer, sequence, CommandStatus::EsmeRinvcmdid).await?;
        }
    }

    Ok(SessionAction::Continue)
}

async fn handle_bind(
    framed: &mut Framed<TcpStream, CommandCodec>,
    peer: SocketAddr,
    state: &mut BindState,
    config: &Config,
    kind: BindKind,
    system_id: &str,
    password: &str,
    interface_version: InterfaceVersion,
    sequence: u32,
) -> Result<(), SessionError> {
    if *state != BindState::Unbound {
        let response = bind_response(kind, config, CommandStatus::EsmeRalybnd, sequence);
        send_command(framed, peer, response).await?;
        return Ok(());
    }

    let auth_ok = config.authenticate(system_id, password);

    let status = if auth_ok {
        *state = match kind {
            BindKind::Transmitter => BindState::Transmitter,
            BindKind::Receiver => BindState::Receiver,
            BindKind::Transceiver => BindState::Transceiver,
        };
        CommandStatus::EsmeRok
    } else {
        warn!(system_id, "bind authentication failed");
        CommandStatus::EsmeRbindfail
    };

    debug!(
        system_id,
        ?interface_version,
        ?status,
        "bind request processed"
    );

    let response = bind_response(kind, config, status, sequence);
    send_command(framed, peer, response).await?;
    Ok(())
}

fn bind_response(kind: BindKind, config: &Config, status: CommandStatus, sequence: u32) -> Command {
    let system_id = config.server_system_id.clone();
    let resp_pdu = match kind {
        BindKind::Transmitter => {
            let resp = BindTransmitterResp::builder()
                .system_id(system_id)
                .sc_interface_version(Some(InterfaceVersion::Smpp5_0))
                .build();
            Pdu::BindTransmitterResp(resp)
        }
        BindKind::Receiver => {
            let resp = BindReceiverResp::builder()
                .system_id(system_id)
                .sc_interface_version(Some(InterfaceVersion::Smpp5_0))
                .build();
            Pdu::BindReceiverResp(resp)
        }
        BindKind::Transceiver => {
            let resp = BindTransceiverResp::builder()
                .system_id(system_id)
                .sc_interface_version(Some(InterfaceVersion::Smpp5_0))
                .build();
            Pdu::BindTransceiverResp(resp)
        }
    };

    Command::new(status, sequence, resp_pdu)
}

async fn send_nack(
    framed: &mut Framed<TcpStream, CommandCodec>,
    peer: SocketAddr,
    sequence: u32,
    status: CommandStatus,
) -> Result<(), SessionError> {
    let response = Command::new(status, sequence, Pdu::GenericNack);
    send_command(framed, peer, response).await
}

async fn send_command(
    framed: &mut Framed<TcpStream, CommandCodec>,
    peer: SocketAddr,
    command: Command,
) -> Result<(), SessionError> {
    info!(peer = %peer, command = ?command, "tx");
    framed.send(command).await.map_err(SessionError::Encode)
}
