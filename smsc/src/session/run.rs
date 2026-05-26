use std::net::SocketAddr;
use std::sync::Arc;

use futures::StreamExt;
use rusmpp::tokio_codec::CommandCodec;
use rusmpp::{Command, CommandStatus};
use tokio::net::TcpStream;
use tokio::sync::broadcast::error::RecvError;
use tokio::time::{sleep, Instant};
use tokio_util::codec::Framed;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::config::Config;
use crate::queue::MessageQueue;

use super::delivery::build_deliver_sm;
use super::handler::handle_command;
use super::io::send_command;
use super::{next_sequence_number, BindState, SessionAction, SessionError};

pub async fn run_session(
    stream: TcpStream,
    peer: SocketAddr,
    config: Arc<Config>,
    queue: Arc<dyn MessageQueue>,
    session_token: CancellationToken,
) -> Result<(), SessionError> {
    let codec = CommandCodec::new().with_max_length(config.max_pdu_length);
    let mut framed = Framed::new(stream, codec);
    let mut state = BindState::Unbound;
    let mut deliveries = queue.subscribe();
    let mut deliveries_open = true;
    let mut next_sequence: u32 = 1;
    let idle_timer = sleep(config.idle_timeout);
    tokio::pin!(idle_timer);

    info!(peer = %peer, "SMPP session started");

    loop {
        tokio::select! {
            _ = session_token.cancelled() => {
                info!(peer = %peer, "server shutdown, closing session");
                return Ok(());
            }
            _ = &mut idle_timer => {
                info!(peer = %peer, "idle timeout, closing session");
                return Ok(());
            }
            maybe_delivery = deliveries.recv(), if deliveries_open => {
                match maybe_delivery {
                    Ok(message) => {
                        if state.allows_rx() {
                            let deliver = build_deliver_sm(&message);
                            let sequence = next_sequence_number(&mut next_sequence);
                            let response = Command::new(CommandStatus::EsmeRok, sequence, deliver);
                            send_command(&mut framed, peer, response).await?;
                        } else {
                            debug!(
                                peer = %peer,
                                message_id = message.message_id.as_str(),
                                "queue delivery skipped; session not bound for rx"
                            );
                        }
                    }
                    Err(RecvError::Lagged(count)) => {
                        warn!(peer = %peer, lagged = count, "queue delivery lagged");
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
                idle_timer.as_mut().reset(Instant::now() + config.idle_timeout);

                if command.id().is_response() {
                    debug!(peer = %peer, command_id = ?command.id(), "ignoring response PDU");
                    continue;
                }

                let action = handle_command(
                    &mut framed,
                    peer,
                    &mut state,
                    &config,
                    &queue,
                    &mut next_sequence,
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
