"""Thin JSON-RPC 2.0 client over Unix socket to Aletheon daemon."""

import asyncio
import json
import os
import time
from typing import Optional


def default_socket_path() -> str:
    runtime_dir = os.environ.get("XDG_RUNTIME_DIR") or f"/run/user/{os.geteuid()}"
    return os.path.join(runtime_dir, "aletheon", "aletheon.sock")


class AletheonClient:
    """JSON-RPC 2.0 client communicating with the daemon over a Unix socket.

    Single connection, reused across all tool calls. Auto-reconnects on
    broken pipe. Timeout handled per call.
    """

    def __init__(
        self,
        socket_path: Optional[str] = None,
        timeout: float = 5.0,
    ):
        self.socket_path = socket_path or os.environ.get(
            "ALETHEON_SOCKET", default_socket_path()
        )
        self.timeout = float(
            os.environ.get("ALETHEON_TIMEOUT", str(timeout))
        )
        self._reader: Optional[asyncio.StreamReader] = None
        self._writer: Optional[asyncio.StreamWriter] = None
        self._lock = asyncio.Lock()
        self._request_id = 0

    async def _connect(self) -> None:
        """Establish (or re-establish) the Unix socket connection."""
        if self._writer is not None:
            try:
                self._writer.close()
            except Exception:
                pass
            try:
                await asyncio.wait_for(self._writer.wait_closed(), timeout=1.0)
            except Exception:
                pass

        self._reader, self._writer = await asyncio.wait_for(
            asyncio.open_unix_connection(self.socket_path),
            timeout=self.timeout,
        )
        # Bump readline buffer limit (default 64KB) for large JSON-RPC responses
        # like session.journal with full message history
        self._reader._limit = 10 * 1024 * 1024  # 10 MB

    def _next_id(self) -> int:
        self._request_id += 1
        return self._request_id

    async def rpc(self, method: str, params: Optional[dict] = None) -> dict:
        """Send a JSON-RPC 2.0 request and return the result or error dict.

        Args:
            method: JSON-RPC method name (e.g. "health", "session.snapshot").
            params: Optional parameters dict.

        Returns:
            Parsed response dict. Always has at least a ``jsonrpc`` and ``id``
            field. On success the ``result`` key is present; on error the
            ``error`` key is present.
        """
        payload = {
            "jsonrpc": "2.0",
            "method": method,
            "params": params or {},
            "id": self._next_id(),
        }
        payload_json = json.dumps(payload, ensure_ascii=False) + "\n"

        async with self._lock:
            # Connect (or reconnect) if needed
            if self._writer is None or self._writer.is_closing():
                await self._connect()

            try:
                self._writer.write(payload_json.encode("utf-8"))
                await asyncio.wait_for(
                    self._writer.drain(), timeout=self.timeout
                )

                line = await asyncio.wait_for(
                    self._reader.readline(), timeout=self.timeout
                )
            except (BrokenPipeError, ConnectionResetError, OSError):
                # Reconnect once, then retry
                await self._connect()
                self._writer.write(payload_json.encode("utf-8"))
                await asyncio.wait_for(
                    self._writer.drain(), timeout=self.timeout
                )
                line = await asyncio.wait_for(
                    self._reader.readline(), timeout=self.timeout
                )

        if not line:
            return {
                "jsonrpc": "2.0",
                "id": payload["id"],
                "error": {
                    "code": -32000,
                    "message": "Daemon closed connection (empty response)",
                },
            }

        try:
            return json.loads(line.decode("utf-8"))
        except json.JSONDecodeError as e:
            return {
                "jsonrpc": "2.0",
                "id": payload["id"],
                "error": {
                    "code": -32700,
                    "message": f"Parse error: {e}",
                    "data": {"raw": line.decode("utf-8", errors="replace")},
                },
            }

    async def close(self) -> None:
        """Close the socket connection."""
        if self._writer is not None:
            try:
                self._writer.close()
                await asyncio.wait_for(
                    self._writer.wait_closed(), timeout=1.0
                )
            except Exception:
                pass
            self._writer = None
            self._reader = None
