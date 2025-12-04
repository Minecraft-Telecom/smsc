from protocols.smpp.server import SMPPServerAdapter
import asyncio

async def main():
    smpp_adapter = SMPPServerAdapter()
    await smpp_adapter.start()
    try:
        while True:
            incoming = await smpp_adapter.receive_sms()
            print(f"Received SMS: {incoming}")
    finally:
        await smpp_adapter.stop()

if __name__ == "__main__":
    asyncio.run(main())
