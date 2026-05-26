use std::net::SocketAddr;

use rusmpp::pdus::{BindReceiverResp, BindTransceiverResp, BindTransmitterResp};
use rusmpp::tokio_codec::CommandCodec;
use rusmpp::values::InterfaceVersion;
use rusmpp::{Command, CommandStatus, Pdu};
use tokio::net::TcpStream;
use tokio_util::codec::Framed;
use tracing::{debug, warn};

use crate::config::Config;

use super::io::send_command;
use super::{BindKind, BindState, SessionError};

pub(super) async fn handle_bind(
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
