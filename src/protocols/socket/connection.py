import asyncio
import logging
from typing import Callable, Coroutine, Any
from dataclasses import dataclass, field

logger = logging.getLogger(__name__)


@dataclass
class Connection:
    """Represents a single client TCP connection."""
    reader: asyncio.StreamReader
    writer: asyncio.StreamWriter
    id: str = field(default_factory=lambda: "")
    _closed: bool = field(default=False, init=False)

    def __post_init__(self):
        if not self.id:
            peername = self.writer.get_extra_info("peername")
            self.id = f"{peername[0]}:{peername[1]}" if peername else "unknown"

    @property
    def is_closed(self) -> bool:
        return self._closed or self.writer.is_closing()

    async def read(self, n: int) -> bytes:
        """Read exactly n bytes from the connection."""
        try:
            data = await self.reader.readexactly(n)
            return data
        except asyncio.IncompleteReadError as e:
            logger.debug(f"Connection {self.id}: incomplete read ({len(e.partial)}/{n} bytes)")
            raise ConnectionError(f"Connection closed while reading") from e
        except ConnectionResetError as e:
            logger.debug(f"Connection {self.id}: reset by peer")
            raise ConnectionError(f"Connection reset") from e

    async def read_until(self, separator: bytes = b"\n") -> bytes:
        """Read until separator is found."""
        try:
            return await self.reader.readuntil(separator)
        except asyncio.IncompleteReadError as e:
            raise ConnectionError(f"Connection closed while reading") from e

    async def read_available(self, max_bytes: int = 4096) -> bytes:
        """Read available data up to max_bytes (non-blocking if data available)."""
        return await self.reader.read(max_bytes)

    async def write(self, data: bytes):
        """Write data to the connection."""
        if self.is_closed:
            raise ConnectionError("Cannot write to closed connection")
        try:
            self.writer.write(data)
            await self.writer.drain()
        except ConnectionResetError as e:
            raise ConnectionError("Connection reset during write") from e

    async def close(self):
        """Close the connection gracefully."""
        if self._closed:
            return
        self._closed = True
        try:
            self.writer.close()
            await self.writer.wait_closed()
        except Exception as e:
            logger.debug(f"Connection {self.id}: error during close: {e}")

    def __repr__(self) -> str:
        status = "closed" if self.is_closed else "open"
        return f"Connection({self.id}, {status})"


ConnectionHandler = Callable[[Connection], Coroutine[Any, Any, None]]
