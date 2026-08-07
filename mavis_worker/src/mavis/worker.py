"""MAVIS AI Worker — UDS server with inference, health checks, and idle unload."""

import asyncio
import json
import logging
import os
import signal
import sys
import time
import uuid
from datetime import datetime, timezone
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent.parent))

from mavis.core.config import load_config
from mavis.core.logger import setup_logging
from mavis.inference.engine import LlamaEngine
from mavis.inference.prompts import build_chat_messages

logger = logging.getLogger(__name__)

SOCKET_PATH = "/tmp/mavis_worker.sock"
IDLE_TIMEOUT = 300  # 5 minutes


class MavisWorker:
    def __init__(self):
        self.config = load_config()
        setup_logging()
        self.engine = LlamaEngine(self.config.data)
        self.last_activity = time.time()
        self._shutdown = asyncio.Event()
        self._idle_task: asyncio.Task | None = None
        self._start_time = time.time()

    async def handle_client(self, reader: asyncio.StreamReader, writer: asyncio.StreamWriter):
        """Handle one UDS connection with length-prefixed JSON."""
        self.last_activity = time.time()
        try:
            while True:
                len_bytes = await reader.readexactly(4)
                length = int.from_bytes(len_bytes, "little")
                data = await reader.readexactly(length)
                request = json.loads(data.decode("utf-8"))

                response = await self.process_request(request)

                resp_bytes = json.dumps(response).encode("utf-8")
                writer.write(len(resp_bytes).to_bytes(4, "little"))
                writer.write(resp_bytes)
                await writer.drain()
                self.last_activity = time.time()
        except asyncio.IncompleteReadError:
            logger.debug("Client disconnected")
        except ConnectionResetError:
            logger.debug("Client connection reset")
        except OSError as e:
            logger.error(f"Client handler OS error: {e}")
        finally:
            writer.close()
            await writer.wait_closed()

    async def process_request(self, request: dict) -> dict:
        """Route incoming requests. Accepts full MAVIS Event dicts or direct requests."""
        event_type = request.get("event_type", request.get("type", "unknown"))
        payload = request.get("payload", request)

        if event_type == "health" or (
            isinstance(payload, dict) and payload.get("type") == "health"
        ):
            return self._make_event(
                "WorkerResponse",
                {
                    "type": "health",
                    "status": "ok",
                    "model_loaded": self.engine.is_loaded,
                    "uptime": time.time() - self._start_time,
                },
            )

        if event_type == "WorkerRequest" or event_type == "chat":
            msg = payload.get("message", payload.get("prompt", ""))
            if not msg:
                return self._error_response("Empty message/prompt")

            await self._ensure_model()
            loop = asyncio.get_event_loop()
            messages = build_chat_messages(msg)
            result = await loop.run_in_executor(
                None,
                lambda: self.engine.chat(
                    messages,
                    max_tokens=payload.get("max_tokens", 512),
                    temperature=payload.get("temperature", 0.7),
                ),
            )
            return self._ok_response(result)

        if event_type == "generate":
            prompt = payload.get("prompt", "")
            if not prompt:
                return self._error_response("Empty prompt")

            await self._ensure_model()
            loop = asyncio.get_event_loop()
            result = await loop.run_in_executor(
                None,
                lambda: self.engine.generate(
                    prompt,
                    max_tokens=payload.get("max_tokens", 512),
                    temperature=payload.get("temperature", 0.7),
                ),
            )
            return self._ok_response(result)

        if event_type == "unload" or (
            isinstance(payload, dict) and payload.get("type") == "unload"
        ):
            self.engine.unload()
            return self._ok_response({"status": "unloaded"})

        if event_type == "memory" or (
            isinstance(payload, dict) and payload.get("type") == "memory"
        ):
            return self._ok_response(self.engine.get_memory_usage())

        return self._error_response(f"Unknown request type: {event_type}")

    def _make_event(self, event_type: str, payload: dict) -> dict:
        return {
            "id": str(uuid.uuid4()),
            "timestamp": datetime.now(timezone.utc).isoformat(),
            "source": "mavis_worker",
            "event_type": event_type,
            "payload": payload,
        }

    def _ok_response(self, result: dict) -> dict:
        return self._make_event(
            "WorkerResponse",
            {
                "type": "response",
                "result": result,
            },
        )

    def _error_response(self, error: str) -> dict:
        return self._make_event(
            "WorkerResponse",
            {
                "type": "error",
                "error": error,
            },
        )

    async def _ensure_model(self):
        """Lazy-load model in thread pool."""
        if not self.engine.is_loaded:
            logger.info("Lazy-loading model...")
            loop = asyncio.get_event_loop()
            await loop.run_in_executor(None, self.engine.load_model)

    async def idle_monitor(self):
        """Unload model after idle timeout to reclaim VRAM."""
        while not self._shutdown.is_set():
            try:
                await asyncio.wait_for(self._shutdown.wait(), timeout=60)
                return
            except asyncio.TimeoutError:
                idle = time.time() - self.last_activity
                if idle > IDLE_TIMEOUT and self.engine.is_loaded:
                    logger.info(f"Idle for {idle:.0f}s — unloading model")
                    self.engine.unload()

    async def run(self):
        """Start UDS server."""
        if os.path.exists(SOCKET_PATH):
            os.remove(SOCKET_PATH)

        server = await asyncio.start_unix_server(self.handle_client, path=SOCKET_PATH)
        os.chmod(SOCKET_PATH, 0o666)
        logger.info(f"MAVIS worker listening on {SOCKET_PATH}")

        self._idle_task = asyncio.create_task(self.idle_monitor())

        for sig in (signal.SIGTERM, signal.SIGINT):
            asyncio.get_event_loop().add_signal_handler(sig, self._shutdown.set)

        await self._shutdown.wait()

        logger.info("Worker shutting down...")
        server.close()
        await server.wait_closed()

        if self._idle_task:
            self._idle_task.cancel()
            try:
                await self._idle_task
            except asyncio.CancelledError:
                pass

        self.engine.unload()
        if os.path.exists(SOCKET_PATH):
            os.remove(SOCKET_PATH)
        logger.info("Worker shutdown complete.")


def main():
    worker = MavisWorker()
    asyncio.run(worker.run())


if __name__ == "__main__":
    main()
