import asyncio
import logging
from protocols.smpp import SMPPServerAdapter

# Configure logging
logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s - %(name)s - %(levelname)s - %(message)s"
)


async def main():
    # Create SMPP server adapter
    smpp_adapter = SMPPServerAdapter(
        host="0.0.0.0",
        port=2775,
        system_id="SMSC"
    )
    
    await smpp_adapter.start()
    print(f"SMPP server running on {smpp_adapter.host}:{smpp_adapter.port}")
    
    try:
        while True:
            # Wait for incoming SMS messages
            incoming = await smpp_adapter.receive_sms()
            print(f"Received SMS: {incoming.sender} -> {incoming.recipient}: {incoming.message}")
            await smpp_adapter.send_sms(incoming)
            
            # Example: Echo the message back (requires a bound receiver)
            # await smpp_adapter.send_sms(incoming)
    except KeyboardInterrupt:
        print("\nShutting down...")
    finally:
        await smpp_adapter.stop()


if __name__ == "__main__":
    asyncio.run(main())
