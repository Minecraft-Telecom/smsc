use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use rusmpp::tokio_codec::CommandCodec;
use rusmpp::{Command, CommandId, CommandStatus, Pdu};
use tokio::net::TcpStream;
use tokio::sync::broadcast::error::RecvError;
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
    let mut state = BindState::Unbound;
    let mut deliveries = queue.subscribe();
    let mut deliveries_open = true;
    let mut next_sequence: u32 = 1;
    let mut bind_failures = 0;
    let mut pending_deliveries = PendingDeliveries::new(config.smpp.deliver_response_timeout);
    let mut lagged_deliveries_total: u64 = 0;
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
                graceful_unbind(&mut framed, peer, &mut next_sequence).await?;
                return Ok(());
            }
            _ = &mut idle_timer => {
                info!(peer = %peer, "idle timeout, closing session");
                return Ok(());
            }
            _ = &mut bind_timer, if state == BindState::Unbound => {
                info!(peer = %peer, "pre-bind timeout, closing session");
                return Ok(());
            }
            _ = delivery_timeout_timer.tick() => {
                pending_deliveries.expire(peer);
            }
            maybe_delivery = deliveries.recv(), if deliveries_open && pending_deliveries.len() < config.smpp.max_pending_deliveries => {
                match maybe_delivery {
                    Ok(message) => {
                        if state.allows_rx() {
                            let deliver = build_deliver_sm(&message);
                            let sequence = next_sequence_number(&mut next_sequence);
                            let response = Command::new(CommandStatus::EsmeRok, sequence, deliver);
                            pending_deliveries
                                .send(
                                    &mut framed,
                                    peer,
                                    response,
                                    DeliverySource::Queue {
                                        message_id: message.message_id.as_str().to_string(),
                                    },
                                )
                                .await?;
                        } else {
                            debug!(
                                peer = %peer,
                                message_id = message.message_id.as_str(),
                                "queue delivery skipped; session not bound for rx"
                            );
                        }
                    }
                    Err(RecvError::Lagged(count)) => {
                        lagged_deliveries_total = lagged_deliveries_total.saturating_add(count);
                        warn!(
                            peer = %peer,
                            lagged = count,
                            lagged_total = lagged_deliveries_total,
                            "queue delivery lagged"
                        );
                    }
                    Err(RecvError::Closed) => {
                        deliveries_open = false;
                        debug!(peer = %peer, "queue delivery channel closed");
                    }
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
                    match pending_deliveries.handle_response(peer, &command) {
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
                    &mut state,
                    &config,
                    &queue,
                    &mut next_sequence,
                    &mut bind_failures,
                    &mut pending_deliveries,
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
