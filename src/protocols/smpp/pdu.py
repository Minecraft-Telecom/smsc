"""SMPP PDU (Protocol Data Unit) definitions."""
from __future__ import annotations

import struct
from dataclasses import dataclass, field
from typing import ClassVar

from .constants import (
    CommandId, CommandStatus, TON, NPI, DataCoding,
    SMPP_HEADER_SIZE, SMPP_VERSION
)


@dataclass
class PDUHeader:
    """SMPP PDU Header (16 bytes)."""
    command_length: int
    command_id: CommandId
    command_status: CommandStatus
    sequence_number: int

    FORMAT: ClassVar[str] = "!IIII"  # 4 x 32-bit unsigned int, big-endian

    def encode(self) -> bytes:
        return struct.pack(
            self.FORMAT,
            self.command_length,
            self.command_id,
            self.command_status,
            self.sequence_number
        )

    @classmethod
    def decode(cls, data: bytes) -> PDUHeader:
        if len(data) < SMPP_HEADER_SIZE:
            raise ValueError(f"Header too short: {len(data)} < {SMPP_HEADER_SIZE}")
        
        cmd_len, cmd_id, cmd_status, seq_num = struct.unpack(cls.FORMAT, data[:SMPP_HEADER_SIZE])
        return cls(
            command_length=cmd_len,
            command_id=CommandId(cmd_id),
            command_status=CommandStatus(cmd_status),
            sequence_number=seq_num
        )


@dataclass
class PDU:
    """Base class for all SMPP PDUs."""
    sequence_number: int = 0
    command_status: CommandStatus = CommandStatus.ESME_ROK

    COMMAND_ID: ClassVar[CommandId]

    def encode(self) -> bytes:
        """Encode the PDU to bytes."""
        body = self._encode_body()
        header = PDUHeader(
            command_length=SMPP_HEADER_SIZE + len(body),
            command_id=self.COMMAND_ID,
            command_status=self.command_status,
            sequence_number=self.sequence_number
        )
        return header.encode() + body

    def _encode_body(self) -> bytes:
        """Encode the PDU body. Override in subclasses."""
        return b""

    @classmethod
    def decode_body(cls, data: bytes, header: PDUHeader) -> PDU:
        """Decode PDU body from bytes. Override in subclasses."""
        raise NotImplementedError


# --- Bind PDUs ---

@dataclass
class BindPDU(PDU):
    """Base class for bind requests."""
    system_id: str = ""
    password: str = ""
    system_type: str = ""
    interface_version: int = SMPP_VERSION
    addr_ton: TON = TON.UNKNOWN
    addr_npi: NPI = NPI.UNKNOWN
    address_range: str = ""

    def _encode_body(self) -> bytes:
        return (
            _encode_c_octet_string(self.system_id, 16) +
            _encode_c_octet_string(self.password, 9) +
            _encode_c_octet_string(self.system_type, 13) +
            struct.pack("!BBB", self.interface_version, self.addr_ton, self.addr_npi) +
            _encode_c_octet_string(self.address_range, 41)
        )

    @classmethod
    def decode_body(cls, data: bytes, header: PDUHeader) -> BindPDU:
        offset = 0
        system_id, offset = _decode_c_octet_string(data, offset)
        password, offset = _decode_c_octet_string(data, offset)
        system_type, offset = _decode_c_octet_string(data, offset)
        interface_version, addr_ton, addr_npi = struct.unpack("!BBB", data[offset:offset+3])
        offset += 3
        address_range, offset = _decode_c_octet_string(data, offset)

        return cls(
            sequence_number=header.sequence_number,
            command_status=header.command_status,
            system_id=system_id,
            password=password,
            system_type=system_type,
            interface_version=interface_version,
            addr_ton=TON(addr_ton),
            addr_npi=NPI(addr_npi),
            address_range=address_range
        )


@dataclass
class BindTransceiver(BindPDU):
    COMMAND_ID: ClassVar[CommandId] = CommandId.BIND_TRANSCEIVER


@dataclass
class BindTransmitter(BindPDU):
    COMMAND_ID: ClassVar[CommandId] = CommandId.BIND_TRANSMITTER


@dataclass
class BindReceiver(BindPDU):
    COMMAND_ID: ClassVar[CommandId] = CommandId.BIND_RECEIVER


@dataclass
class BindResponse(PDU):
    """Base class for bind responses."""
    system_id: str = ""
    sc_interface_version: int | None = None  # Optional TLV

    def _encode_body(self) -> bytes:
        body = _encode_c_octet_string(self.system_id, 16)
        if self.sc_interface_version is not None:
            # TLV: sc_interface_version (tag=0x0210, length=1)
            body += struct.pack("!HHB", 0x0210, 1, self.sc_interface_version)
        return body

    @classmethod
    def decode_body(cls, data: bytes, header: PDUHeader) -> BindResponse:
        offset = 0
        system_id, offset = _decode_c_octet_string(data, offset)
        sc_version = None
        # Parse optional TLVs
        while offset < len(data) - 4:
            tag, length = struct.unpack("!HH", data[offset:offset+4])
            if tag == 0x0210 and length >= 1:
                sc_version = data[offset+4]
            offset += 4 + length
        
        return cls(
            sequence_number=header.sequence_number,
            command_status=header.command_status,
            system_id=system_id,
            sc_interface_version=sc_version
        )


@dataclass
class BindTransceiverResp(BindResponse):
    COMMAND_ID: ClassVar[CommandId] = CommandId.BIND_TRANSCEIVER_RESP


@dataclass
class BindTransmitterResp(BindResponse):
    COMMAND_ID: ClassVar[CommandId] = CommandId.BIND_TRANSMITTER_RESP


@dataclass
class BindReceiverResp(BindResponse):
    COMMAND_ID: ClassVar[CommandId] = CommandId.BIND_RECEIVER_RESP


# --- Unbind PDUs ---

@dataclass
class Unbind(PDU):
    COMMAND_ID: ClassVar[CommandId] = CommandId.UNBIND

    @classmethod
    def decode_body(cls, data: bytes, header: PDUHeader) -> Unbind:
        return cls(sequence_number=header.sequence_number, command_status=header.command_status)


@dataclass
class UnbindResp(PDU):
    COMMAND_ID: ClassVar[CommandId] = CommandId.UNBIND_RESP

    @classmethod
    def decode_body(cls, data: bytes, header: PDUHeader) -> UnbindResp:
        return cls(sequence_number=header.sequence_number, command_status=header.command_status)


# --- Enquire Link PDUs ---

@dataclass
class EnquireLink(PDU):
    COMMAND_ID: ClassVar[CommandId] = CommandId.ENQUIRE_LINK

    @classmethod
    def decode_body(cls, data: bytes, header: PDUHeader) -> EnquireLink:
        return cls(sequence_number=header.sequence_number, command_status=header.command_status)


@dataclass
class EnquireLinkResp(PDU):
    COMMAND_ID: ClassVar[CommandId] = CommandId.ENQUIRE_LINK_RESP

    @classmethod
    def decode_body(cls, data: bytes, header: PDUHeader) -> EnquireLinkResp:
        return cls(sequence_number=header.sequence_number, command_status=header.command_status)


# --- Submit SM PDUs ---

@dataclass
class SubmitSM(PDU):
    """Submit Short Message PDU."""
    COMMAND_ID: ClassVar[CommandId] = CommandId.SUBMIT_SM

    service_type: str = ""
    source_addr_ton: TON = TON.UNKNOWN
    source_addr_npi: NPI = NPI.UNKNOWN
    source_addr: str = ""
    dest_addr_ton: TON = TON.UNKNOWN
    dest_addr_npi: NPI = NPI.UNKNOWN
    destination_addr: str = ""
    esm_class: int = 0
    protocol_id: int = 0
    priority_flag: int = 0
    schedule_delivery_time: str = ""
    validity_period: str = ""
    registered_delivery: int = 0
    replace_if_present_flag: int = 0
    data_coding: DataCoding = DataCoding.DEFAULT
    sm_default_msg_id: int = 0
    short_message: bytes = b""

    def _encode_body(self) -> bytes:
        sm_length = len(self.short_message)
        return (
            _encode_c_octet_string(self.service_type, 6) +
            struct.pack("!BB", self.source_addr_ton, self.source_addr_npi) +
            _encode_c_octet_string(self.source_addr, 21) +
            struct.pack("!BB", self.dest_addr_ton, self.dest_addr_npi) +
            _encode_c_octet_string(self.destination_addr, 21) +
            struct.pack("!BBB", self.esm_class, self.protocol_id, self.priority_flag) +
            _encode_c_octet_string(self.schedule_delivery_time, 17) +
            _encode_c_octet_string(self.validity_period, 17) +
            struct.pack("!BBBB", self.registered_delivery, self.replace_if_present_flag,
                       self.data_coding, self.sm_default_msg_id) +
            struct.pack("!B", sm_length) +
            self.short_message
        )

    @classmethod
    def decode_body(cls, data: bytes, header: PDUHeader) -> SubmitSM:
        offset = 0
        service_type, offset = _decode_c_octet_string(data, offset)
        source_addr_ton, source_addr_npi = struct.unpack("!BB", data[offset:offset+2])
        offset += 2
        source_addr, offset = _decode_c_octet_string(data, offset)
        dest_addr_ton, dest_addr_npi = struct.unpack("!BB", data[offset:offset+2])
        offset += 2
        destination_addr, offset = _decode_c_octet_string(data, offset)
        esm_class, protocol_id, priority_flag = struct.unpack("!BBB", data[offset:offset+3])
        offset += 3
        schedule_delivery_time, offset = _decode_c_octet_string(data, offset)
        validity_period, offset = _decode_c_octet_string(data, offset)
        (registered_delivery, replace_if_present_flag,
         data_coding, sm_default_msg_id) = struct.unpack("!BBBB", data[offset:offset+4])
        offset += 4
        sm_length = data[offset]
        offset += 1
        short_message = data[offset:offset+sm_length]

        return cls(
            sequence_number=header.sequence_number,
            command_status=header.command_status,
            service_type=service_type,
            source_addr_ton=TON(source_addr_ton),
            source_addr_npi=NPI(source_addr_npi),
            source_addr=source_addr,
            dest_addr_ton=TON(dest_addr_ton),
            dest_addr_npi=NPI(dest_addr_npi),
            destination_addr=destination_addr,
            esm_class=esm_class,
            protocol_id=protocol_id,
            priority_flag=priority_flag,
            schedule_delivery_time=schedule_delivery_time,
            validity_period=validity_period,
            registered_delivery=registered_delivery,
            replace_if_present_flag=replace_if_present_flag,
            data_coding=DataCoding(data_coding),
            sm_default_msg_id=sm_default_msg_id,
            short_message=short_message
        )


@dataclass
class SubmitSMResp(PDU):
    """Submit SM Response PDU."""
    COMMAND_ID: ClassVar[CommandId] = CommandId.SUBMIT_SM_RESP
    message_id: str = ""

    def _encode_body(self) -> bytes:
        return _encode_c_octet_string(self.message_id, 65)

    @classmethod
    def decode_body(cls, data: bytes, header: PDUHeader) -> SubmitSMResp:
        message_id, _ = _decode_c_octet_string(data, 0)
        return cls(
            sequence_number=header.sequence_number,
            command_status=header.command_status,
            message_id=message_id
        )


# --- Deliver SM PDUs ---

@dataclass
class DeliverSM(PDU):
    """Deliver Short Message PDU (server -> client)."""
    COMMAND_ID: ClassVar[CommandId] = CommandId.DELIVER_SM

    service_type: str = ""
    source_addr_ton: TON = TON.UNKNOWN
    source_addr_npi: NPI = NPI.UNKNOWN
    source_addr: str = ""
    dest_addr_ton: TON = TON.UNKNOWN
    dest_addr_npi: NPI = NPI.UNKNOWN
    destination_addr: str = ""
    esm_class: int = 0
    protocol_id: int = 0
    priority_flag: int = 0
    schedule_delivery_time: str = ""
    validity_period: str = ""
    registered_delivery: int = 0
    replace_if_present_flag: int = 0
    data_coding: DataCoding = DataCoding.DEFAULT
    sm_default_msg_id: int = 0
    short_message: bytes = b""

    def _encode_body(self) -> bytes:
        sm_length = len(self.short_message)
        return (
            _encode_c_octet_string(self.service_type, 6) +
            struct.pack("!BB", self.source_addr_ton, self.source_addr_npi) +
            _encode_c_octet_string(self.source_addr, 21) +
            struct.pack("!BB", self.dest_addr_ton, self.dest_addr_npi) +
            _encode_c_octet_string(self.destination_addr, 21) +
            struct.pack("!BBB", self.esm_class, self.protocol_id, self.priority_flag) +
            _encode_c_octet_string(self.schedule_delivery_time, 17) +
            _encode_c_octet_string(self.validity_period, 17) +
            struct.pack("!BBBB", self.registered_delivery, self.replace_if_present_flag,
                       self.data_coding, self.sm_default_msg_id) +
            struct.pack("!B", sm_length) +
            self.short_message
        )

    @classmethod
    def decode_body(cls, data: bytes, header: PDUHeader) -> DeliverSM:
        offset = 0
        service_type, offset = _decode_c_octet_string(data, offset)
        source_addr_ton, source_addr_npi = struct.unpack("!BB", data[offset:offset+2])
        offset += 2
        source_addr, offset = _decode_c_octet_string(data, offset)
        dest_addr_ton, dest_addr_npi = struct.unpack("!BB", data[offset:offset+2])
        offset += 2
        destination_addr, offset = _decode_c_octet_string(data, offset)
        esm_class, protocol_id, priority_flag = struct.unpack("!BBB", data[offset:offset+3])
        offset += 3
        schedule_delivery_time, offset = _decode_c_octet_string(data, offset)
        validity_period, offset = _decode_c_octet_string(data, offset)
        (registered_delivery, replace_if_present_flag,
         data_coding, sm_default_msg_id) = struct.unpack("!BBBB", data[offset:offset+4])
        offset += 4
        sm_length = data[offset]
        offset += 1
        short_message = data[offset:offset+sm_length]

        return cls(
            sequence_number=header.sequence_number,
            command_status=header.command_status,
            service_type=service_type,
            source_addr_ton=TON(source_addr_ton),
            source_addr_npi=NPI(source_addr_npi),
            source_addr=source_addr,
            dest_addr_ton=TON(dest_addr_ton),
            dest_addr_npi=NPI(dest_addr_npi),
            destination_addr=destination_addr,
            esm_class=esm_class,
            protocol_id=protocol_id,
            priority_flag=priority_flag,
            schedule_delivery_time=schedule_delivery_time,
            validity_period=validity_period,
            registered_delivery=registered_delivery,
            replace_if_present_flag=replace_if_present_flag,
            data_coding=DataCoding(data_coding),
            sm_default_msg_id=sm_default_msg_id,
            short_message=short_message
        )


@dataclass
class DeliverSMResp(PDU):
    """Deliver SM Response PDU."""
    COMMAND_ID: ClassVar[CommandId] = CommandId.DELIVER_SM_RESP
    message_id: str = ""

    def _encode_body(self) -> bytes:
        # Usually NULL for deliver_sm_resp
        return _encode_c_octet_string(self.message_id, 65)

    @classmethod
    def decode_body(cls, data: bytes, header: PDUHeader) -> DeliverSMResp:
        message_id = ""
        if data:
            message_id, _ = _decode_c_octet_string(data, 0)
        return cls(
            sequence_number=header.sequence_number,
            command_status=header.command_status,
            message_id=message_id
        )


# --- Generic NACK ---

@dataclass
class GenericNack(PDU):
    """Generic Negative Acknowledgement."""
    COMMAND_ID: ClassVar[CommandId] = CommandId.GENERIC_NACK

    @classmethod
    def decode_body(cls, data: bytes, header: PDUHeader) -> GenericNack:
        return cls(sequence_number=header.sequence_number, command_status=header.command_status)


# --- Helper functions ---

def _encode_c_octet_string(value: str, max_length: int) -> bytes:
    """Encode a C-Octet String (null-terminated)."""
    encoded = value.encode("latin-1")[:max_length - 1]
    return encoded + b"\x00"


def _decode_c_octet_string(data: bytes, offset: int) -> tuple[str, int]:
    """Decode a C-Octet String from data at offset. Returns (string, new_offset)."""
    end = data.index(b"\x00", offset)
    value = data[offset:end].decode("latin-1")
    return value, end + 1
