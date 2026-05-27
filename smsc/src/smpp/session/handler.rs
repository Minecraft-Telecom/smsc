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
use super::deliver::DeliverySource;
use super::delivery::{build_delivery_receipt, wants_delivery_receipt};
use super::io::{send_command, send_nack};
use super::{BindKind, BindOutcome, SessionAction, SessionData, SessionError, next_sequence_number};

pub(super) async fn handle_command(
    framed: &mut Framed<TcpStream, CommandCodec>,
    peer: SocketAddr,
    session_data: &mut SessionData,
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
            let params = super::bind::BindParams {
                kind: BindKind::Transmitter,
                system_id: bind.system_id.as_str(),
                password: bind.password.as_str(),
                interface_version: bind.interface_version,
            };
            let outcome = handle_bind(
                framed,
                peer,
                &mut session_data.state, config,
                params,
                sequence,
            )
            .await?;
            return handle_bind_outcome(peer, config, &mut session_data.bind_failures, outcome);
        }
        Pdu::BindReceiver(bind) => {
            let params = super::bind::BindParams {
                kind: BindKind::Receiver,
                system_id: bind.system_id.as_str(),
                password: bind.password.as_str(),
                interface_version: bind.interface_version,
            };
            let outcome = handle_bind(
                framed,
                peer,
                &mut session_data.state, config,
                params,
                sequence,
            )
            .await?;
            return handle_bind_outcome(peer, config, &mut session_data.bind_failures, outcome);
        }
        Pdu::BindTransceiver(bind) => {
            let params = super::bind::BindParams {
                kind: BindKind::Transceiver,
                system_id: bind.system_id.as_str(),
                password: bind.password.as_str(),
                interface_version: bind.interface_version,
            };
            let outcome = handle_bind(
                framed,
                peer,
                &mut session_data.state, config,
                params,
                sequence,
            )
            .await?;
            return handle_bind_outcome(peer, config, &mut session_data.bind_failures, outcome);
        }
        Pdu::EnquireLink => {
            let response = Command::new(CommandStatus::EsmeRok, sequence, Pdu::EnquireLinkResp);
            send_command(framed, peer, response).await?;
        }
        Pdu::SubmitSm(submit) => {
            if !session_data.state.allows_tx() {
                send_nack(framed, peer, sequence, CommandStatus::EsmeRinvbndsts).await?;
                return Ok(SessionAction::Continue);
            }

            let wants_receipt = wants_delivery_receipt(submit);
            let message_opt = match queue.enqueue(submit) {
                Ok(msg) => Some(msg),
                Err(_) => {
                    warn!("submit_sm rejected by queue");
                    None
                }
            };

            let (status, message_id) = match &message_opt {
                Some(msg) => (CommandStatus::EsmeRok, msg.message_id()),
                None => (
                    CommandStatus::EsmeRsubmitfail,
                    COctetString::<1, 65>::empty(),
                ),
            };

            let resp = SubmitSmResp::builder()
                .message_id(message_id.clone())
                .build();
            let response = Command::new(status, sequence, resp);
            send_command(framed, peer, response).await?;

            if status == CommandStatus::EsmeRok && wants_receipt {
                let msg = message_opt.unwrap();
                if session_data.state.allows_rx() {
                    if session_data.pending_deliveries.len() >= config.smpp.max_pending_deliveries {
                        warn!(
                            peer = %peer,
                            message_id = msg.message_id_str(),
                            "delivery receipt skipped; pending delivery window full"
                        );
                    } else {
                        let deliver = build_delivery_receipt(&msg)?;
                        let sequence = next_sequence_number(&mut session_data.next_sequence);
                        let receipt = Command::new(CommandStatus::EsmeRok, sequence, deliver);
                        session_data.pending_deliveries
                            .send(
                                framed,
                                peer,
                                receipt,
                                DeliverySource::Receipt {
                                    message_id: msg.message_id_str().to_string(),
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
            if *bind_failures >= config.smpp.max_bind_failures {
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



