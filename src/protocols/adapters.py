from utils import SMSMessage, SMSQueue

class AbstractProtocolAdapter:
    def __init__(self):
        self.sms_queue = SMSQueue()

    async def send_sms(self, sms: SMSMessage):
        await self.sms_queue.send(sms)

    async def receive_sms(self) -> SMSMessage:
        return await self.sms_queue.receive()
    
    def start(self):
        raise NotImplementedError("Subclasses must implement this method.")
    
    def stop(self):
        raise NotImplementedError("Subclasses must implement this method.")
