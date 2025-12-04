"""SMPP Server Adapter - integrates TCP server with SMPP protocol handling."""
from __future__ import annotations

import asyncio
import datetime
import logging
import time
from dataclasses import dataclass, field
from typing import Callable, Coroutine, Any

from utils import SMSMessage, SMSEvent, EventType, EventQueue
from ..adapters import AbstractProtocolAdapter
from ..socket import TCPServer, Connection
from .session import SMPPSession
from .pdu import SubmitSM, BindPDU
from .constants import DataCoding, TON, NPI

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
    
    # Global event queue for cross-adapter delivery reports
    event_queue: EventQueue | None = None
    
    # Internal state
    _server: TCPServer | None = field(default=None, init=False)
    _sessions: dict[str, SMPPSession] = field(default_factory=dict, init=False)
    _outgoing_task: asyncio.Task | None = field(default=None, init=False)
    _event_task: asyncio.Task | None = field(default=None, init=False)
    _running: bool = field(default=False, init=False)
    _pending_delivery_reports: dict[str, dict] = field(default_factory=dict, init=False)

    def __post_init__(self):
        # Initialize parent class
        super().__init__(self.event_queue)

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
        
        # Start event processor for delivery reports from other adapters
        if self.event_queue:
            self._event_task = asyncio.create_task(self._process_events())
        
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

        # Stop event processor
        if self._event_task:
            self._event_task.cancel()
            try:
                await self._event_task
            except asyncio.CancelledError:
                pass
            self._event_task = None

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

    async def _on_submit_sm(self, session: SMPPSession, pdu: SubmitSM) -> str | None:
        """Handle incoming SMS from client (submit_sm). Returns message_id."""
        # Decode message based on data coding
        message_text = self._decode_message(pdu.short_message, pdu.data_coding)
        
        sms = SMSMessage(
            sender=pdu.source_addr,
            recipient=pdu.destination_addr,
            message=message_text,
            sent_time=time.time()
        )
        
        logger.info(f"Received SMS [{sms.message_id}]: {pdu.source_addr} -> {pdu.destination_addr}: {message_text[:50]}...")
        
        # Store session info for delivery reports if requested
        if pdu.registered_delivery & 0x01:  # Bit 0 indicates delivery receipt requested
            self._pending_delivery_reports[sms.message_id] = {
                'session_id': session.connection.id,
                'source': pdu.source_addr,
                'destination': pdu.destination_addr,
                'source_ton': pdu.source_addr_ton,
                'source_npi': pdu.source_addr_npi,
                'dest_ton': pdu.dest_addr_ton,
                'dest_npi': pdu.dest_addr_npi,
                'submit_time': sms.sent_time,
                'message': message_text,
            }
        
        # Put in incoming queue
        await self.sms_queue.incoming.put(sms)
        
        return sms.message_id

    async def _process_outgoing(self):
        """Process outgoing SMS messages and deliver to connected clients."""
        # Track retry attempts per message
        retry_counts: dict[str, int] = {}
        max_retries = 3
        
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
                            logger.info(f"Delivered SMS [{sms.message_id}] to {session.client_system_id}: {sms.sender} -> {sms.recipient}")
                            delivered = True
                            retry_counts.pop(sms.message_id, None)
                            
                            # Emit delivery event for other adapters
                            if self.event_queue:
                                await self.event_queue.emit(SMSEvent(
                                    event_type=EventType.MESSAGE_DELIVERED,
                                    message_id=sms.message_id,
                                    data={
                                        'delivered_time': sms.delivered_time,
                                        'source': sms.sender,
                                        'destination': sms.recipient,
                                        'message': sms.message
                                    }
                                ))
                            break
                    except Exception as e:
                        logger.error(f"Failed to deliver to {session.connection.id}: {e}")

                if not delivered:
                    # Track retries
                    retries = retry_counts.get(sms.message_id, 0) + 1
                    retry_counts[sms.message_id] = retries
                    
                    if retries >= max_retries:
                        logger.error(f"Failed to deliver SMS [{sms.message_id}] after {max_retries} attempts")
                        retry_counts.pop(sms.message_id, None)
                        
                        # Emit failure event
                        if self.event_queue:
                            await self.event_queue.emit(SMSEvent(
                                event_type=EventType.MESSAGE_FAILED,
                                message_id=sms.message_id,
                                data={
                                    'failed_time': time.time(),
                                    'source': sms.sender,
                                    'destination': sms.recipient,
                                    'message': sms.message,
                                    'error_code': '001',
                                    'reason': 'No available session to deliver message'
                                }
                            ))
                    else:
                        logger.warning(f"No available session to deliver SMS [{sms.message_id}] to {sms.recipient} (attempt {retries}/{max_retries})")
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

    async def _process_events(self):
        """Process events from the global event queue."""
        if not self.event_queue:
            return
        
        try:
            while self._running:
                try:
                    event = await asyncio.wait_for(
                        self.event_queue.get(),
                        timeout=1.0
                    )
                except asyncio.TimeoutError:
                    continue
                
                if event.event_type == EventType.MESSAGE_DELIVERED:
                    await self._handle_delivery_event(event)
                elif event.event_type == EventType.MESSAGE_FAILED:
                    await self._handle_failure_event(event)
        except asyncio.CancelledError:
            pass

    async def _handle_delivery_event(self, event: SMSEvent):
        """Handle a MESSAGE_DELIVERED event by sending delivery report."""
        report_info = self._pending_delivery_reports.pop(event.message_id, None)
        if not report_info:
            return  # No delivery report requested for this message
        
        await self._send_delivery_report(
            message_id=event.message_id,
            report_info=report_info,
            delivered_time=event.data.get('delivered_time', time.time()),
            status="DELIVRD",
            error_code="000",
            message_text=event.data.get('message', '')[:20]
        )

    async def _handle_failure_event(self, event: SMSEvent):
        """Handle a MESSAGE_FAILED event by sending failure report."""
        report_info = self._pending_delivery_reports.pop(event.message_id, None)
        if not report_info:
            return
        
        await self._send_delivery_report(
            message_id=event.message_id,
            report_info=report_info,
            delivered_time=event.data.get('failed_time', time.time()),
            status="UNDELIV",
            error_code=event.data.get('error_code', '001'),
            message_text=event.data.get('message', '')[:20]
        )

    async def _send_delivery_report(
        self,
        message_id: str,
        report_info: dict,
        delivered_time: float,
        status: str,
        error_code: str,
        message_text: str
    ):
        """Send delivery report (DLR) to the originating session."""
        
        session = self._sessions.get(report_info['session_id'])
        if not session or not session.can_receive:
            logger.warning(f"Cannot send DLR for {message_id}: session unavailable")
            return
        
        # Format delivery report message per SMPP spec
        # Format: id:IIIIIIIIII sub:SSS dlvrd:DDD submit date:YYMMDDhhmm done date:YYMMDDhhmm stat:DDDDDDD err:E text:...
        submit_time = datetime.datetime.fromtimestamp(report_info['submit_time']).strftime("%y%m%d%H%M")
        done_time = datetime.datetime.fromtimestamp(delivered_time).strftime("%y%m%d%H%M")
        
        dlr_text = (
            f"id:{message_id} "
            f"sub:001 "
            f"dlvrd:{'001' if status == 'DELIVRD' else '000'} "
            f"submit date:{submit_time} "
            f"done date:{done_time} "
            f"stat:{status} "
            f"err:{error_code} "
            f"text:{message_text}"
        )
        
        try:
            # Delivery reports are sent with esm_class = 0x04 (delivery receipt)
            success = await session.deliver_message(
                source=report_info['destination'],  # Swap source/dest for DLR
                destination=report_info['source'],
                message=dlr_text.encode('latin-1'),
                data_coding=DataCoding.DEFAULT,
                source_ton=report_info['dest_ton'],
                source_npi=report_info['dest_npi'],
                dest_ton=report_info['source_ton'],
                dest_npi=report_info['source_npi'],
                esm_class=0x04  # Delivery receipt
            )
            if success:
                logger.info(f"Sent delivery report for message {message_id}")
            else:
                logger.warning(f"Failed to send delivery report for {message_id}")
        except Exception as e:
            logger.error(f"Error sending delivery report for {message_id}: {e}")

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
