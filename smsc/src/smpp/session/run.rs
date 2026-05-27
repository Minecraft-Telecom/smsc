use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use rusmpp::tokio_codec::CommandCodec;
use rusmpp::{Command, CommandId, CommandStatus, Pdu};
use tokio::net::TcpStream;
use tokio::time::{Instant, interval, sleep, timeout};
use tokio_util::codec::Framed;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::config::Config;
use crate::queue::MessageQueue;

use super::deliver::{DeliverySource, PendingDeliveries};
use super::delivery::build_deliver_sm;
use super::handler::handle_command;
use super::io::send_command;
use super::{BindState, SessionAction, SessionError, next_sequence_number};

pub async fn run_session(
    stream: TcpStream,
    peer: SocketAddr,
    config: Arc<Config>,
    queue: Arc<dyn MessageQueue>,
    session_token: CancellationToken,
) -> Result<(), SessionError> {
    let codec = CommandCodec::new().with_max_length(config.smpp.max_pdu_length);
    let mut framed = Framed::new(stream, codec);
    let mut session_data = super::SessionData { state: BindState::Unbound, next_sequence: 1, bind_failures: 0, pending_deliveries: PendingDeliveries::new(config.smpp.deliver_response_timeout) };
    
    
    
    let idle_timer = sleep(config.smpp.idle_timeout);
    let bind_timer = sleep(config.smpp.bind_timeout);
    let mut delivery_timeout_timer = interval(config.smpp.deliver_response_timeout);
    tokio::pin!(idle_timer);
    tokio::pin!(bind_timer);

    info!(peer = %peer, "SMPP session started");

    loop {
        tokio::select! {
            _ = session_token.cancelled() => {
                info!(peer = %peer, "server shutdown, sending unbind");
                graceful_unbind(&mut framed, peer, &mut session_data.next_sequence).await?;
                return Ok(());
            }
            _ = &mut idle_timer => {
                info!(peer = %peer, "idle timeout, closing session");
                return Ok(());
            }
            _ = &mut bind_timer, if session_data.state == BindState::Unbound => {
                info!(peer = %peer, "pre-bind timeout, closing session");
                return Ok(());
            }
            _ = delivery_timeout_timer.tick() => {
                session_data.pending_deliveries.expire(peer);
            }
            message = queue.dequeue(), if session_data.pending_deliveries.len() < config.smpp.max_pending_deliveries => {
                if session_data.state.allows_rx() {
                    let deliver = build_deliver_sm(&message);
                    let sequence = next_sequence_number(&mut session_data.next_sequence);
                    let response = Command::new(CommandStatus::EsmeRok, sequence, deliver);
                    session_data.pending_deliveries.send(
                            &mut framed,
                            peer,
                            response,
                            DeliverySource::Queue {
                                message_id: message.message_id_str().to_string(),
                            },
                        )
                        .await?;
                } else {
                    debug!(
                        peer = %peer,
                        message_id = message.message_id_str(),
                        "queue delivery skipped; session not bound for rx"
                    );
                    // Currently we drop the message since we popped it but cannot rx.
                    // A production ready queue would maybe requeue or nack.
                    queue.update_status(message.message_id_str(), crate::queue::MessageStatus::Failed);
                }
            }
            maybe_command = framed.next() => {
                let command = match maybe_command {
                    Some(Ok(command)) => command,
                    Some(Err(err)) => return Err(SessionError::Decode(err)),
                    None => {
                        info!(peer = %peer, "peer disconnected");
                        return Ok(());
                    }
                };

                info!(peer = %peer, command_id = ?command.id(), sequence = command.sequence_number(), "rx");
                debug!(peer = %peer, command = ?command, "rx detail");
                idle_timer.as_mut().reset(Instant::now() + config.smpp.idle_timeout);

                if command.id().is_response() {
                    match session_data.pending_deliveries.handle_response(peer, &command) {
                        Some((source, status)) => {
                            if let super::deliver::DeliverySource::Queue { message_id } = source {
                                let message_status = if status == rusmpp::CommandStatus::EsmeRok {
                                    crate::queue::MessageStatus::Delivered
                                } else {
                                    crate::queue::MessageStatus::Failed
                                };
                                queue.update_status(&message_id, message_status);
                            }
                        }
                        None => {
                            debug!(peer = %peer, command_id = ?command.id(), "ignoring response PDU");
                        }
                    }
                    continue;
                }

                let action = handle_command(
                    &mut framed,
                    peer,
                    &mut session_data,
                    &config,
                    &queue,
                    command,
                )
                .await?;
                if action == SessionAction::Close {
                    info!(peer = %peer, "session closed by peer");
                    break;
                }
            }
        }
    }

    Ok(())
}

async fn graceful_unbind(
    framed: &mut Framed<TcpStream, CommandCodec>,
    peer: SocketAddr,
    next_sequence: &mut u32,
) -> Result<(), SessionError> {
    let sequence = next_sequence_number(next_sequence);
    let unbind = Command::new(CommandStatus::EsmeRok, sequence, Pdu::Unbind);
    send_command(framed, peer, unbind).await?;

    let _ = timeout(Duration::from_secs(5), async {
        while let Some(result) = framed.next().await {
            match result {
                Ok(command) if command.id() == CommandId::UnbindResp => break,
                Ok(command) => {
                    debug!(
                        peer = %peer,
                        command_id = ?command.id(),
                        "waiting for unbind_resp"
                    );
                }
                Err(err) => {
                    warn!(peer = %peer, ?err, "decode error while waiting for unbind_resp");
                    break;
                }
            }
        }
    })
    .await;

    Ok(())
}

