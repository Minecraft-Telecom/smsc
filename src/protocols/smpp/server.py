"""SMPP Server Adapter - integrates TCP server with SMPP protocol handling."""
from __future__ import annotations

import asyncio
import logging
import time
from dataclasses import dataclass, field
from typing import Callable, Coroutine, Any

from utils import SMSMessage
from ..adapters import AbstractProtocolAdapter
from ..socket import TCPServer, Connection
from .session import SMPPSession
from .pdu import SubmitSM, BindPDU
from .constants import DataCoding

logger = logging.getLogger(__name__)

# Type alias for authentication callback
AuthCallback = Callable[[str, str], Coroutine[Any, Any, bool]]


@dataclass
class SMPPServerAdapter(AbstractProtocolAdapter):
    """
    SMPP Server adapter that handles real TCP connections.
    
    Accepts SMPP client connections, handles binding, and routes
    SMS messages between clients and the internal queue system.
    """
    host: str = "0.0.0.0"
    port: int = 2775
    system_id: str = "SMSC"
    
    # Authentication callback (optional)
    # Should return True if credentials are valid
    auth_callback: AuthCallback | None = None
    
    # Internal state
    _server: TCPServer | None = field(default=None, init=False)
    _sessions: dict[str, SMPPSession] = field(default_factory=dict, init=False)
    _outgoing_task: asyncio.Task | None = field(default=None, init=False)
    _running: bool = field(default=False, init=False)

    def __post_init__(self):
        # Initialize parent class
        super().__init__()

    async def start(self):
        """Start the SMPP server."""
        if self._running:
            logger.warning("SMPP server is already running")
            return

        self._running = True
        
        # Create TCP server
        self._server = TCPServer(
            host=self.host,
            port=self.port,
            handler=self._handle_connection
        )
        
        await self._server.start()
        
        # Start outgoing message processor
        self._outgoing_task = asyncio.create_task(self._process_outgoing())
        
        logger.info(f"SMPP server started on {self.host}:{self.port}")

    async def stop(self):
        """Stop the SMPP server."""
        if not self._running:
            return

        self._running = False
        logger.info("Stopping SMPP server...")

        # Stop outgoing processor
        if self._outgoing_task:
            self._outgoing_task.cancel()
            try:
                await self._outgoing_task
            except asyncio.CancelledError:
                pass
            self._outgoing_task = None

        # Stop TCP server (this will close all connections)
        if self._server:
            await self._server.stop()
            self._server = None

        self._sessions.clear()
        logger.info("SMPP server stopped")

    async def _handle_connection(self, connection: Connection):
        """Handle a new client connection."""
        session = SMPPSession(
            connection=connection,
            system_id=self.system_id,
            on_message=self._on_submit_sm,
            on_bind=self._on_bind
        )
        
        self._sessions[connection.id] = session
        
        try:
            await session.run()
        finally:
            self._sessions.pop(connection.id, None)

    async def _on_bind(self, bind_pdu: BindPDU) -> bool:
        """Handle bind authentication."""
        if self.auth_callback:
            return await self.auth_callback(
                bind_pdu.system_id,
                bind_pdu.password
            )
        # Accept all binds if no auth callback
        return True

    async def _on_submit_sm(self, session: SMPPSession, pdu: SubmitSM):
        """Handle incoming SMS from client (submit_sm)."""
        # Decode message based on data coding
        message_text = self._decode_message(pdu.short_message, pdu.data_coding)
        
        sms = SMSMessage(
            sender=pdu.source_addr,
            recipient=pdu.destination_addr,
            message=message_text,
            sent_time=time.time()
        )
        
        logger.info(f"Received SMS: {pdu.source_addr} -> {pdu.destination_addr}: {message_text[:50]}...")
        
        # Put in incoming queue
        await self.sms_queue.incoming.put(sms)

    async def _process_outgoing(self):
        """Process outgoing SMS messages and deliver to connected clients."""
        try:
            while self._running:
                try:
                    # Wait for outgoing message with timeout for shutdown check
                    sms = await asyncio.wait_for(
                        self.sms_queue.outgoing.get(),
                        timeout=1.0
                    )
                except asyncio.TimeoutError:
                    continue

                # Find a session that can receive messages
                delivered = False
                for session in self._get_receiving_sessions():
                    try:
                        # Encode message
                        message_bytes, data_coding = self._encode_message(sms.message)
                        
                        success = await session.deliver_message(
                            source=sms.sender,
                            destination=sms.recipient,
                            message=message_bytes,
                            data_coding=data_coding
                        )
                        
                        if success:
                            sms.delivered_time = time.time()
                            logger.info(f"Delivered SMS to {session.client_system_id}: {sms.sender} -> {sms.recipient}")
                            delivered = True
                            break
                    except Exception as e:
                        logger.error(f"Failed to deliver to {session.connection.id}: {e}")

                if not delivered:
                    logger.warning(f"No available session to deliver SMS to {sms.recipient}")
                    # Put back in queue for retry
                    await self.sms_queue.outgoing.put(sms)
                    await asyncio.sleep(1.0)  # Brief delay before retry

        except asyncio.CancelledError:
            pass

    def _get_receiving_sessions(self) -> list[SMPPSession]:
        """Get all sessions that can receive messages."""
        return [s for s in self._sessions.values() if s.can_receive]

    def _get_transmitting_sessions(self) -> list[SMPPSession]:
        """Get all sessions that can transmit messages."""
        return [s for s in self._sessions.values() if s.can_transmit]

    @staticmethod
    def _decode_message(data: bytes, data_coding: DataCoding) -> str:
        """Decode message bytes based on data coding scheme."""
        if data_coding == DataCoding.UCS2:
            return data.decode("utf-16-be")
        elif data_coding in (DataCoding.LATIN1, DataCoding.IA5):
            return data.decode("latin-1")
        elif data_coding in (DataCoding.BINARY, DataCoding.BINARY_8BIT):
            return data.hex()
        else:
            # Default: try GSM 7-bit as ASCII, fallback to latin-1
            try:
                return data.decode("ascii")
            except UnicodeDecodeError:
                return data.decode("latin-1")

    @staticmethod
    def _encode_message(text: str) -> tuple[bytes, DataCoding]:
        """Encode message text to bytes, returning (bytes, data_coding)."""
        try:
            # Try ASCII first (most compatible)
            return text.encode("ascii"), DataCoding.DEFAULT
        except UnicodeEncodeError:
            # Fall back to UCS2 for unicode
            return text.encode("utf-16-be"), DataCoding.UCS2

    @property
    def is_running(self) -> bool:
        return self._running

    @property
    def connection_count(self) -> int:
        return len(self._sessions)

    @property
    def bound_sessions(self) -> list[SMPPSession]:
        return [s for s in self._sessions.values() if s.is_bound]
