import asyncio
import logging
from dataclasses import dataclass, field

from .connection import Connection, ConnectionHandler

logger = logging.getLogger(__name__)


@dataclass
class TCPServer:
    """A robust async TCP server with connection management."""
    host: str = "0.0.0.0"
    port: int = 2775  # Default SMPP port
    handler: ConnectionHandler | None = None
    _server: asyncio.Server | None = field(default=None, init=False)
    _connections: dict[str, Connection] = field(default_factory=dict, init=False)
    _running: bool = field(default=False, init=False)
    _connection_tasks: dict[str, asyncio.Task] = field(default_factory=dict, init=False)

    async def _handle_client(self, reader: asyncio.StreamReader, writer: asyncio.StreamWriter):
        """Internal callback for new connections."""
        conn = Connection(reader=reader, writer=writer)
        self._connections[conn.id] = conn
        logger.info(f"New connection: {conn.id}")

        try:
            if self.handler:
                task = asyncio.create_task(self.handler(conn))
                self._connection_tasks[conn.id] = task
                await task
        except asyncio.CancelledError:
            logger.debug(f"Connection {conn.id}: handler cancelled")
        except Exception as e:
            logger.error(f"Connection {conn.id}: handler error: {e}")
        finally:
            await conn.close()
            self._connections.pop(conn.id, None)
            self._connection_tasks.pop(conn.id, None)
            logger.info(f"Connection closed: {conn.id}")

    async def start(self):
        """Start the TCP server."""
        if self._running:
            logger.warning("Server is already running")
            return

        self._server = await asyncio.start_server(
            self._handle_client,
            self.host,
            self.port,
            reuse_address=True
        )
        self._running = True

        addrs = ", ".join(str(sock.getsockname()) for sock in self._server.sockets)
        logger.info(f"TCP server started on {addrs}")

    async def serve_forever(self):
        """Start the server and run until stopped."""
        await self.start()
        if self._server:
            async with self._server:
                await self._server.serve_forever()

    async def stop(self):
        """Stop the server and close all connections."""
        if not self._running:
            return

        self._running = False
        logger.info("Stopping TCP server...")

        # Cancel all connection handlers
        for task in self._connection_tasks.values():
            task.cancel()

        # Wait for all handlers to finish
        if self._connection_tasks:
            await asyncio.gather(*self._connection_tasks.values(), return_exceptions=True)

        # Close all connections
        for conn in list(self._connections.values()):
            await conn.close()

        # Close the server
        if self._server:
            self._server.close()
            await self._server.wait_closed()
            self._server = None

        logger.info("TCP server stopped")

    @property
    def is_running(self) -> bool:
        return self._running

    @property
    def connection_count(self) -> int:
        return len(self._connections)

    def get_connections(self) -> list[Connection]:
        return list(self._connections.values())
