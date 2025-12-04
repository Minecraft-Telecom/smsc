from dataclasses import dataclass
from asyncio import Queue

@dataclass
class SMSMessage:
    recipient: str
    sender: str
    message: str
    sent_time: float
    delivered_time: float | None = None

class SMSQueue:
    def __init__(self):
        self.outgoing: Queue[SMSMessage] = Queue()
        self.incoming: Queue[SMSMessage] = Queue()

    async def send(self, sms: SMSMessage):
        await self.outgoing.put(sms)

    async def receive(self) -> SMSMessage:
        return await self.incoming.get()