from utils import SMSMessage, SMSQueue, EventQueue

class AbstractProtocolAdapter:
    def __init__(self, event_queue: EventQueue | None = None):
        self.sms_queue = SMSQueue()
        self.event_queue = event_queue

    async def send_sms(self, sms: SMSMessage):
        await self.sms_queue.send(sms)

    async def receive_sms(self) -> SMSMessage:
        return await self.sms_queue.receive()
    
    def start(self):
        raise NotImplementedError("Subclasses must implement this method.")
    
    def stop(self):
        raise NotImplementedError("Subclasses must implement this method.")
