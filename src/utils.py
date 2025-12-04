from dataclasses import dataclass, field
from asyncio import Queue
from enum import Enum, auto
from typing import Any
import uuid


def _generate_message_id() -> str:
    """Generate a unique message ID."""
    return uuid.uuid4().hex[:16]


class EventType(Enum):
    """Types of events that can occur in the SMS system."""
    MESSAGE_SUBMITTED = auto()   # Message received from sender
    MESSAGE_DELIVERED = auto()   # Message successfully delivered to recipient
    MESSAGE_FAILED = auto()      # Message delivery failed
    MESSAGE_EXPIRED = auto()     # Message expired before delivery


@dataclass
class SMSEvent:
    """An event related to an SMS message."""
    event_type: EventType
    message_id: str
    # Additional event-specific data
    data: dict[str, Any] = field(default_factory=dict)


@dataclass
class SMSMessage:
    recipient: str
    sender: str
    message: str
    sent_time: float
    message_id: str = field(default_factory=_generate_message_id)
    delivered_time: float | None = None


class EventQueue:
    """Global event queue for SMS lifecycle events."""
    def __init__(self):
        self._queue: Queue[SMSEvent] = Queue()
    
    async def emit(self, event: SMSEvent):
        """Emit an event to the queue."""
        await self._queue.put(event)
    
    async def get(self) -> SMSEvent:
        """Get the next event from the queue."""
        return await self._queue.get()
    
    def get_nowait(self) -> SMSEvent | None:
        """Get an event without waiting, returns None if empty."""
        try:
            return self._queue.get_nowait()
        except:
            return None


class SMSQueue:
    def __init__(self):
        self.outgoing: Queue[SMSMessage] = Queue()
        self.incoming: Queue[SMSMessage] = Queue()

    async def send(self, sms: SMSMessage):
        await self.outgoing.put(sms)

    async def receive(self) -> SMSMessage:
        return await self.incoming.get()