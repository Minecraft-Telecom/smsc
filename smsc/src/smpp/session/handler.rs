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
use super::deliver::{DeliverySource, PendingDeliveries};
use super::delivery::{build_delivery_receipt, wants_delivery_receipt};
use super::io::{send_command, send_nack};
use super::{BindKind, BindOutcome, BindState, SessionAction, SessionError, next_sequence_number};

pub(super) async fn handle_command(
    framed: &mut Framed<TcpStream, CommandCodec>,
    peer: SocketAddr,
    state: &mut BindState,
    config: &Config,
    queue: &Arc<dyn MessageQueue>,
    next_sequence: &mut u32,
    bind_failures: &mut usize,
    pending_deliveries: &mut PendingDeliveries,
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
            let outcome = handle_bind(
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
            return handle_bind_outcome(peer, config, bind_failures, outcome);
        }
        Pdu::BindReceiver(bind) => {
            let outcome = handle_bind(
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
            return handle_bind_outcome(peer, config, bind_failures, outcome);
        }
        Pdu::BindTransceiver(bind) => {
            let outcome = handle_bind(
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
            return handle_bind_outcome(peer, config, bind_failures, outcome);
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
            let (status, message_id) = match queue.enqueue(&submit) {
                Ok(message_id) => (CommandStatus::EsmeRok, message_id),
                Err(_) => {
                    warn!("submit_sm rejected by queue");
                    (
                        CommandStatus::EsmeRsubmitfail,
                        COctetString::<1, 65>::empty(),
                    )
                }
            };

            let resp = SubmitSmResp::builder()
                .message_id(message_id.clone())
                .build();
            let response = Command::new(status, sequence, resp);
            send_command(framed, peer, response).await?;

            if status == CommandStatus::EsmeRok && wants_receipt {
                if state.allows_rx() {
                    if pending_deliveries.len() >= config.max_pending_deliveries {
                        warn!(
                            peer = %peer,
                            message_id = message_id.as_str(),
                            "delivery receipt skipped; pending delivery window full"
                        );
                    } else {
                        let deliver = build_delivery_receipt(&submit, &message_id)?;
                        let sequence = next_sequence_number(next_sequence);
                        let receipt = Command::new(CommandStatus::EsmeRok, sequence, deliver);
                        pending_deliveries
                            .send(
                                framed,
                                peer,
                                receipt,
                                DeliverySource::Receipt {
                                    message_id: message_id.as_str().to_string(),
                                },
                            )
                            .await?;
                    }
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
        // Responses are filtered in run.rs before handle_command is called.
        _ => {
            send_nack(framed, peer, sequence, CommandStatus::EsmeRinvcmdid).await?;
        }
    }

    Ok(SessionAction::Continue)
}

fn handle_bind_outcome(
    peer: SocketAddr,
    config: &Config,
    bind_failures: &mut usize,
    outcome: BindOutcome,
) -> Result<SessionAction, SessionError> {
    match outcome {
        BindOutcome::Accepted => {
            *bind_failures = 0;
            Ok(SessionAction::Continue)
        }
        BindOutcome::AlreadyBound => Ok(SessionAction::Continue),
        BindOutcome::Rejected => {
            *bind_failures += 1;
            if *bind_failures >= config.max_bind_failures {
                warn!(
                    peer = %peer,
                    failures = *bind_failures,
                    "closing session after repeated bind authentication failures"
                );
                Ok(SessionAction::Close)
            } else {
                Ok(SessionAction::Continue)
            }
        }
    }
}
