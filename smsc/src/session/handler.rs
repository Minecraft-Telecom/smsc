use std::net::SocketAddr;
use std::sync::Arc;

use rusmpp::pdus::SubmitSmResp;
use rusmpp::tokio_codec::CommandCodec;
use rusmpp::types::COctetString;
use rusmpp::{Command, CommandStatus, Pdu};
use tokio::net::TcpStream;
use tokio_util::codec::Framed;
use tracing::{debug, warn};

use crate::config::Config;
use crate::queue::MessageQueue;

use super::bind::handle_bind;
use super::delivery::{build_delivery_receipt, wants_delivery_receipt};
use super::io::{send_command, send_nack};
use super::{next_sequence_number, BindKind, BindState, SessionAction, SessionError};

pub(super) async fn handle_command(
    framed: &mut Framed<TcpStream, CommandCodec>,
    peer: SocketAddr,
    state: &mut BindState,
    config: &Config,
    queue: &Arc<dyn MessageQueue>,
    next_sequence: &mut u32,
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

            let wants_receipt = wants_delivery_receipt(&submit);
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

            let resp = SubmitSmResp::builder()
                .message_id(message_id.clone())
                .build();
            let response = Command::new(status, sequence, resp);
            send_command(framed, peer, response).await?;

            if status == CommandStatus::EsmeRok && wants_receipt {
                if state.allows_rx() {
                    let deliver = build_delivery_receipt(&submit, &message_id);
                    let sequence = next_sequence_number(next_sequence);
                    let receipt = Command::new(CommandStatus::EsmeRok, sequence, deliver);
                    send_command(framed, peer, receipt).await?;
                } else {
                    debug!(peer = %peer, "delivery receipt skipped; session not bound for rx");
                }
            }
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
