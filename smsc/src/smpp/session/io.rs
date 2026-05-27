use std::net::SocketAddr;

use futures::SinkExt;
use rusmpp::tokio_codec::CommandCodec;
use rusmpp::{Command, CommandStatus, Pdu};
use tokio::net::TcpStream;
use tokio_util::codec::Framed;
use tracing::info;

use super::SessionError;

pub(super) async fn send_nack(
    framed: &mut Framed<TcpStream, CommandCodec>,
    peer: SocketAddr,
    sequence: u32,
    status: CommandStatus,
) -> Result<(), SessionError> {
    let response = Command::new(status, sequence, Pdu::GenericNack);
    send_command(framed, peer, response).await
}

pub(super) async fn send_command(
    framed: &mut Framed<TcpStream, CommandCodec>,
    peer: SocketAddr,
    command: Command,
) -> Result<(), SessionError> {
    info!(peer = %peer, command_id = ?command.id(), sequence = command.sequence_number(), "tx");
    tracing::debug!(peer = %peer, command = ?command, "tx detail");
    framed.send(command).await.map_err(SessionError::Encode)
}
