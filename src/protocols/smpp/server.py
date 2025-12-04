import time
import asyncio
import random
from utils import SMSMessage
from ..adapters import AbstractProtocolAdapter

class SMPPServerAdapter(AbstractProtocolAdapter):
    def __init__(self):
        super().__init__()
        self._task: asyncio.Task | None = None
        self._stop_event = asyncio.Event()

    async def _run_server(self):
        """Internal producer task for generating incoming SMS."""
        try:
            while not self._stop_event.is_set():
                await asyncio.sleep(random.uniform(0.5, 2.0))
                sms = SMSMessage(
                    sender=f"+123456789{random.randint(0,9)}",
                    recipient="+9876543210",
                    message="Hello from SMPP!",
                    sent_time=time.time()
                )
                await self.sms_queue.incoming.put(sms)
        except asyncio.CancelledError:
            # Clean exit when task is cancelled
            pass

    async def start(self):
        """Start the internal SMPP task."""
        if self._task is None or self._task.done():
            self._stop_event.clear()
            self._task = asyncio.create_task(self._run_server())

    async def stop(self):
        """Stop the internal SMPP task."""
        if self._task:
            self._stop_event.set()
            self._task.cancel()
            try:
                await self._task
            except asyncio.CancelledError:
                pass
            self._task = None
