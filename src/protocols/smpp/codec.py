"""SMPP Codec - PDU encoding and decoding."""
from __future__ import annotations

import logging
from typing import Type

from .constants import CommandId, CommandStatus, SMPP_HEADER_SIZE, SMPP_MAX_PDU_SIZE
from .pdu import (
    PDU, PDUHeader,
    BindTransceiver, BindTransceiverResp,
    BindTransmitter, BindTransmitterResp,
    BindReceiver, BindReceiverResp,
    Unbind, UnbindResp,
    EnquireLink, EnquireLinkResp,
    SubmitSM, SubmitSMResp,
    DeliverSM, DeliverSMResp,
    GenericNack,
)

logger = logging.getLogger(__name__)


# Registry of PDU classes by command ID
PDU_REGISTRY: dict[CommandId, Type[PDU]] = {
    CommandId.BIND_TRANSCEIVER: BindTransceiver,
    CommandId.BIND_TRANSCEIVER_RESP: BindTransceiverResp,
    CommandId.BIND_TRANSMITTER: BindTransmitter,
    CommandId.BIND_TRANSMITTER_RESP: BindTransmitterResp,
    CommandId.BIND_RECEIVER: BindReceiver,
    CommandId.BIND_RECEIVER_RESP: BindReceiverResp,
    CommandId.UNBIND: Unbind,
    CommandId.UNBIND_RESP: UnbindResp,
    CommandId.ENQUIRE_LINK: EnquireLink,
    CommandId.ENQUIRE_LINK_RESP: EnquireLinkResp,
    CommandId.SUBMIT_SM: SubmitSM,
    CommandId.SUBMIT_SM_RESP: SubmitSMResp,
    CommandId.DELIVER_SM: DeliverSM,
    CommandId.DELIVER_SM_RESP: DeliverSMResp,
    CommandId.GENERIC_NACK: GenericNack,
}


class SMPPDecodeError(Exception):
    """Raised when PDU decoding fails."""
    pass


class SMPPCodec:
    """Encoder and decoder for SMPP PDUs."""

    @staticmethod
    def encode(pdu: PDU) -> bytes:
        """Encode a PDU to bytes."""
        return pdu.encode()

    @staticmethod
    def decode_header(data: bytes) -> PDUHeader:
        """Decode just the PDU header from bytes."""
        if len(data) < SMPP_HEADER_SIZE:
            raise SMPPDecodeError(f"Not enough data for header: {len(data)} < {SMPP_HEADER_SIZE}")
        return PDUHeader.decode(data)

    @staticmethod
    def decode(data: bytes) -> PDU:
        """Decode a complete PDU from bytes."""
        if len(data) < SMPP_HEADER_SIZE:
            raise SMPPDecodeError(f"Not enough data for PDU: {len(data)} < {SMPP_HEADER_SIZE}")

        header = PDUHeader.decode(data)
        
        if header.command_length > SMPP_MAX_PDU_SIZE:
            raise SMPPDecodeError(f"PDU too large: {header.command_length} > {SMPP_MAX_PDU_SIZE}")
        
        if len(data) < header.command_length:
            raise SMPPDecodeError(
                f"Incomplete PDU: have {len(data)} bytes, need {header.command_length}"
            )

        # Get the body (everything after header)
        body = data[SMPP_HEADER_SIZE:header.command_length]

        # Look up the PDU class
        try:
            command_id = CommandId(header.command_id)
        except ValueError:
            raise SMPPDecodeError(f"Unknown command ID: 0x{header.command_id:08X}")

        pdu_class = PDU_REGISTRY.get(command_id)
        if pdu_class is None:
            raise SMPPDecodeError(f"Unsupported command: {command_id.name}")

        try:
            return pdu_class.decode_body(body, header)
        except Exception as e:
            raise SMPPDecodeError(f"Failed to decode {command_id.name}: {e}") from e

    @staticmethod
    def get_pdu_length(data: bytes) -> int | None:
        """
        Get the expected length of a PDU from the header.
        Returns None if not enough data to read header.
        """
        if len(data) < 4:
            return None
        import struct
        return struct.unpack("!I", data[:4])[0]


def create_bind_response(bind_pdu: PDU, system_id: str = "SMSC",
                         status: CommandStatus = CommandStatus.ESME_ROK) -> PDU:
    """Create appropriate bind response for a bind request."""
    response_map = {
        CommandId.BIND_TRANSCEIVER: BindTransceiverResp,
        CommandId.BIND_TRANSMITTER: BindTransmitterResp,
        CommandId.BIND_RECEIVER: BindReceiverResp,
    }
    
    resp_class = response_map.get(bind_pdu.COMMAND_ID)
    if resp_class is None:
        raise ValueError(f"Cannot create response for {bind_pdu.COMMAND_ID}")
    
    return resp_class(
        sequence_number=bind_pdu.sequence_number,
        command_status=status,
        system_id=system_id
    )
