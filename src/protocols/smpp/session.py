"""SMPP Session management for handling client connections."""
from __future__ import annotations

import asyncio
import logging
import time
from dataclasses import dataclass, field
from enum import Enum, auto
from typing import Callable, Coroutine, Any

from ..socket.connection import Connection
from .codec import SMPPCodec, SMPPDecodeError, create_bind_response
from .constants import CommandId, CommandStatus, SMPP_HEADER_SIZE, TON, NPI, DataCoding
from .pdu import (
    PDU, BindPDU, BindTransceiver, BindTransmitter, BindReceiver,
    Unbind, UnbindResp, EnquireLink, EnquireLinkResp,
    SubmitSM, SubmitSMResp, DeliverSM, DeliverSMResp, GenericNack
)

logger = logging.getLogger(__name__)


class SessionState(Enum):
    """SMPP session states."""
    OPEN = auto()       # Connected, not bound
    BOUND_TX = auto()   # Bound as transmitter
    BOUND_RX = auto()   # Bound as receiver
    BOUND_TRX = auto()  # Bound as transceiver
    UNBOUND = auto()    # Unbound, closing
    CLOSED = auto()     # Disconnected


# Type alias for message handlers
# Returns optional message_id string (for submit_sm response)
MessageHandler = Callable[["SMPPSession", SubmitSM], Coroutine[Any, Any, str | None]]


@dataclass
class SMPPSession:
    """
    Manages an SMPP session over a TCP connection.
    
    Handles binding, message exchange, and session lifecycle.
    """
    connection: Connection
    system_id: str = "SMSC"
    enquire_link_timeout: float = 30.0
    response_timeout: float = 10.0
    
    # Callbacks
    on_message: MessageHandler | None = None
    on_bind: Callable[[BindPDU], Coroutine[Any, Any, bool]] | None = None
    
    # Internal state
    state: SessionState = field(default=SessionState.OPEN, init=False)
    client_system_id: str = field(default="", init=False)
    _buffer: bytes = field(default=b"", init=False)
    _sequence: int = field(default=0, init=False)
    _pending_responses: dict[int, asyncio.Future] = field(default_factory=dict, init=False)
    _last_activity: float = field(default_factory=time.time, init=False)
    _tasks: list[asyncio.Task] = field(default_factory=list, init=False)

    def _next_sequence(self) -> int:
        """Get next sequence number."""
        self._sequence += 1
        return self._sequence

    async def run(self):
        """Main session loop - read and process PDUs."""
        logger.info(f"Session started for {self.connection.id}")
        
        try:
            # Start keepalive task
            keepalive_task = asyncio.create_task(self._keepalive_loop())
            self._tasks.append(keepalive_task)
            
            while self.state != SessionState.CLOSED:
                try:
                    pdu = await self._read_pdu()
                    if pdu is None:
                        break
                    await self._handle_pdu(pdu)
                except ConnectionError as e:
                    logger.info(f"Session {self.connection.id}: connection closed: {e}")
                    break
                except SMPPDecodeError as e:
                    logger.warning(f"Session {self.connection.id}: decode error: {e}")
                    await self._send_generic_nack(CommandStatus.ESME_RINVCMDID, 0)
                except Exception as e:
                    logger.error(f"Session {self.connection.id}: error: {e}")
                    break
        finally:
            await self._cleanup()

    async def _read_pdu(self) -> PDU | None:
        """Read a complete PDU from the connection."""
        # Read header if we don't have enough data
        while len(self._buffer) < SMPP_HEADER_SIZE:
            data = await self.connection.read_available(4096)
            if not data:
                return None
            self._buffer += data
            self._last_activity = time.time()

        # Get PDU length from header
        pdu_length = SMPPCodec.get_pdu_length(self._buffer)
        if pdu_length is None:
            return None

        # Read rest of PDU
        while len(self._buffer) < pdu_length:
            data = await self.connection.read_available(4096)
            if not data:
                return None
            self._buffer += data
            self._last_activity = time.time()

        # Decode PDU
        pdu_data = self._buffer[:pdu_length]
        self._buffer = self._buffer[pdu_length:]
        
        return SMPPCodec.decode(pdu_data)

    async def _handle_pdu(self, pdu: PDU):
        """Route PDU to appropriate handler."""
        logger.debug(f"Session {self.connection.id}: received {pdu.COMMAND_ID.name}")

        # Check for pending response
        if pdu.sequence_number in self._pending_responses:
            future = self._pending_responses.pop(pdu.sequence_number)
            future.set_result(pdu)
            return

        # Handle by command type
        if isinstance(pdu, (BindTransceiver, BindTransmitter, BindReceiver)):
            await self._handle_bind(pdu)
        elif isinstance(pdu, Unbind):
            await self._handle_unbind(pdu)
        elif isinstance(pdu, EnquireLink):
            await self._handle_enquire_link(pdu)
        elif isinstance(pdu, SubmitSM):
            await self._handle_submit_sm(pdu)
        elif isinstance(pdu, DeliverSMResp):
            # Response to our deliver_sm - already handled above
            pass
        else:
            logger.warning(f"Unhandled PDU type: {pdu.COMMAND_ID.name}")
            await self._send_generic_nack(CommandStatus.ESME_RINVCMDID, pdu.sequence_number)

    async def _handle_bind(self, pdu: BindPDU):
        """Handle bind request."""
        if self.state != SessionState.OPEN:
            await self._send_response(create_bind_response(
                pdu, self.system_id, CommandStatus.ESME_RALYBND
            ))
            return

        # Check credentials via callback if provided
        if self.on_bind:
            allowed = await self.on_bind(pdu)
            if not allowed:
                await self._send_response(create_bind_response(
                    pdu, self.system_id, CommandStatus.ESME_RBINDFAIL
                ))
                return

        self.client_system_id = pdu.system_id
        
        # Set state based on bind type
        if isinstance(pdu, BindTransceiver):
            self.state = SessionState.BOUND_TRX
        elif isinstance(pdu, BindTransmitter):
            self.state = SessionState.BOUND_TX
        elif isinstance(pdu, BindReceiver):
            self.state = SessionState.BOUND_RX

        logger.info(f"Session {self.connection.id}: bound as {self.state.name} ({self.client_system_id})")
        
        await self._send_response(create_bind_response(pdu, self.system_id))

    async def _handle_unbind(self, pdu: Unbind):
        """Handle unbind request."""
        self.state = SessionState.UNBOUND
        await self._send_response(UnbindResp(sequence_number=pdu.sequence_number))
        logger.info(f"Session {self.connection.id}: unbound")

    async def _handle_enquire_link(self, pdu: EnquireLink):
        """Handle enquire_link (keepalive)."""
        await self._send_response(EnquireLinkResp(sequence_number=pdu.sequence_number))

    async def _handle_submit_sm(self, pdu: SubmitSM):
        """Handle submit_sm (incoming message from client)."""
        if self.state not in (SessionState.BOUND_TX, SessionState.BOUND_TRX):
            await self._send_response(SubmitSMResp(
                sequence_number=pdu.sequence_number,
                command_status=CommandStatus.ESME_RINVBNDSTS
            ))
            return

        message_id = ""

        # Notify handler and get message ID
        if self.on_message:
            try:
                message_id = await self.on_message(self, pdu) or ""
            except Exception as e:
                logger.error(f"Message handler error: {e}")
                await self._send_response(SubmitSMResp(
                    sequence_number=pdu.sequence_number,
                    command_status=CommandStatus.ESME_RSYSERR,
                    message_id=""
                ))
                return

        # Send success response with message ID
        await self._send_response(SubmitSMResp(
            sequence_number=pdu.sequence_number,
            message_id=message_id
        ))

    async def deliver_message(self, source: str, destination: str, message: bytes,
                              data_coding: DataCoding = DataCoding.DEFAULT,
                              source_ton: TON = TON.INTERNATIONAL,
                              source_npi: NPI = NPI.ISDN,
                              dest_ton: TON = TON.INTERNATIONAL,
                              dest_npi: NPI = NPI.ISDN,
                              esm_class: int = 0) -> bool:
        """
        Send a message to the connected client (deliver_sm).
        
        Args:
            esm_class: ESM class field. Use 0x04 for delivery receipts.
        
        Returns True if acknowledged successfully.
        """
        if self.state not in (SessionState.BOUND_RX, SessionState.BOUND_TRX):
            logger.warning(f"Cannot deliver: session not bound for receiving")
            return False

        pdu = DeliverSM(
            sequence_number=self._next_sequence(),
            source_addr=source,
            source_addr_ton=source_ton,
            source_addr_npi=source_npi,
            destination_addr=destination,
            dest_addr_ton=dest_ton,
            dest_addr_npi=dest_npi,
            short_message=message,
            data_coding=data_coding,
            esm_class=esm_class
        )

        try:
            response = await self._send_and_wait(pdu)
            if isinstance(response, DeliverSMResp):
                return response.command_status == CommandStatus.ESME_ROK
            return False
        except asyncio.TimeoutError:
            logger.warning(f"Deliver timeout for session {self.connection.id}")
            return False

    async def _send_response(self, pdu: PDU):
        """Send a PDU response."""
        data = SMPPCodec.encode(pdu)
        await self.connection.write(data)
        logger.debug(f"Session {self.connection.id}: sent {pdu.COMMAND_ID.name}")

    async def _send_and_wait(self, pdu: PDU, timeout: float | None = None) -> PDU:
        """Send a PDU and wait for response."""
        if timeout is None:
            timeout = self.response_timeout

        future: asyncio.Future[PDU] = asyncio.get_event_loop().create_future()
        self._pending_responses[pdu.sequence_number] = future

        try:
            await self._send_response(pdu)
            return await asyncio.wait_for(future, timeout)
        except asyncio.TimeoutError:
            self._pending_responses.pop(pdu.sequence_number, None)
            raise

    async def _send_generic_nack(self, status: CommandStatus, sequence: int):
        """Send a generic_nack."""
        nack = GenericNack(sequence_number=sequence, command_status=status)
        await self._send_response(nack)

    async def _keepalive_loop(self):
        """Send enquire_link periodically to keep session alive."""
        try:
            while self.state not in (SessionState.CLOSED, SessionState.UNBOUND):
                await asyncio.sleep(self.enquire_link_timeout)
                
                # Check if we need to send keepalive
                if time.time() - self._last_activity > self.enquire_link_timeout:
                    if self.state in (SessionState.BOUND_TX, SessionState.BOUND_RX, SessionState.BOUND_TRX):
                        try:
                            pdu = EnquireLink(sequence_number=self._next_sequence())
                            await self._send_and_wait(pdu, timeout=10.0)
                            self._last_activity = time.time()
                        except asyncio.TimeoutError:
                            logger.warning(f"Session {self.connection.id}: keepalive timeout")
                            break
        except asyncio.CancelledError:
            pass

    async def _cleanup(self):
        """Clean up session resources."""
        self.state = SessionState.CLOSED
        
        # Cancel tasks
        for task in self._tasks:
            task.cancel()
        
        # Cancel pending responses
        for future in self._pending_responses.values():
            future.cancel()
        self._pending_responses.clear()
        
        logger.info(f"Session {self.connection.id}: closed")

    @property
    def is_bound(self) -> bool:
        return self.state in (SessionState.BOUND_TX, SessionState.BOUND_RX, SessionState.BOUND_TRX)

    @property
    def can_receive(self) -> bool:
        return self.state in (SessionState.BOUND_RX, SessionState.BOUND_TRX)

    @property
    def can_transmit(self) -> bool:
        return self.state in (SessionState.BOUND_TX, SessionState.BOUND_TRX)
