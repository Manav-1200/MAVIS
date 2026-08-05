"""
MAVIS AI Worker — Unix domain socket server.

Receives WorkerRequest events from the Rust core,
runs AI inference (loaded lazily), and sends WorkerResponse events back.
"""

from __future__ import annotations

import asyncio
import json
import logging
import sys
import uuid
from datetime import UTC, datetime
from pathlib import Path

# --- Immediate logging setup (before any heavy imports) ---
logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [%(name)s] %(levelname)s: %(message)s",
    handlers=[logging.StreamHandler(sys.stdout)],
)
logger = logging.getLogger("mavis.worker")

SOCKET_PATH = Path("/tmp/mavis_worker.sock")

# Lazy-loaded AI modules (torch/transformers are heavy)
_ai_modules: dict[str, object] = {}


def _load_ai() -> dict[str, object]:
    """Lazily import torch and transformers."""
    if "torch" not in _ai_modules:
        logger.info("Loading AI modules (torch, transformers)...")
        import torch
        import transformers

        _ai_modules["torch"] = torch
        _ai_modules["transformers"] = transformers
        logger.info("AI modules loaded. CUDA available: %s", torch.cuda.is_available())
    return _ai_modules


def _make_event(event_type: str, payload: dict) -> dict:
    """Build a canonical MAVIS Event."""
    return {
        "id": str(uuid.uuid4()),
        "timestamp": datetime.now(UTC).isoformat(),
        "source": "mavis_worker",
        "event_type": event_type,
        "payload": payload,
    }


async def _handle_request(raw: str) -> str | None:
    """Parse a request, run inference, return a response JSON line."""
    try:
        event = json.loads(raw)
    except json.JSONDecodeError as exc:
        logger.error("Invalid JSON from core: %s", exc)
        return None

    event_type = event.get("event_type")
    payload = event.get("payload", {})
    logger.info("Received %s: %s", event_type, payload)

    if event_type == "WorkerRequest":
        task = payload.get("task", "unknown")
        prompt = payload.get("prompt", "")

        # Lazy-load AI only when actually needed
        _load_ai()

        # STUB: Replace with actual model inference
        result = f"[STUB] Task '{task}' on prompt: {prompt[:50]}..."

        response = _make_event(
            "WorkerResponse",
            {
                "request_id": event.get("id"),
                "task": task,
                "result": result,
            },
        )
        return json.dumps(response) + "\n"

    logger.warning("Unhandled event type: %s", event_type)
    return None


async def _client_handler(
    reader: asyncio.StreamReader,
    writer: asyncio.StreamWriter,
) -> None:
    """Handle a single connection from the Rust core."""
    peer = writer.get_extra_info("peername")
    logger.info("Rust core connected: %s", peer)

    try:
        while True:
            line = await reader.readline()
            if not line:
                break

            raw = line.decode("utf-8").strip()
            if not raw:
                continue

            response = await _handle_request(raw)
            if response:
                writer.write(response.encode("utf-8"))
                await writer.drain()
    except asyncio.CancelledError:
        raise
    except Exception as exc:
        logger.error("Client handler error: %s", exc)
    finally:
        writer.close()
        await writer.wait_closed()
        logger.info("Rust core disconnected: %s", peer)


async def run_worker() -> None:
    """Start the UDS server and serve until interrupted."""
    logger.info("Starting MAVIS Worker...")

    # Clean up stale socket
    if SOCKET_PATH.exists():
        SOCKET_PATH.unlink()
        logger.info("Removed stale socket: %s", SOCKET_PATH)

    server = await asyncio.start_unix_server(
        _client_handler,
        path=str(SOCKET_PATH),
    )

    logger.info("MAVIS Worker listening on %s", SOCKET_PATH)
    logger.info("Waiting for Rust core to connect...")

    async with server:
        await server.serve_forever()


def main() -> int:
    """Entry point for the worker process."""
    try:
        asyncio.run(run_worker())
    except KeyboardInterrupt:
        logger.info("Worker shutting down (Ctrl+C).")
    finally:
        if SOCKET_PATH.exists():
            SOCKET_PATH.unlink()
            logger.info("Cleaned up socket: %s", SOCKET_PATH)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
