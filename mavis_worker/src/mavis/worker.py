import asyncio
import gc
import json
import os
import signal
import struct
import threading
import time
import uuid
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timezone

from mavis.core.config import load_config
from mavis.inference import SYSTEM_PROMPT, LlamaEngine, build_chat_messages


def _now_iso() -> str:
    return datetime.now(timezone.utc).isoformat()


def _make_event(payload: dict, event_type: str = "WorkerResponse") -> dict:
    return {
        "id": str(uuid.uuid4()),
        "timestamp": _now_iso(),
        "source": "mavis_worker",
        "event_type": event_type,
        "payload": payload,
    }


class WorkerServer:
    def __init__(self, socket_path: str = "/tmp/mavis_worker.sock"):
        self.socket_path = socket_path
        self.engine = LlamaEngine()
        self.config = load_config()
        self.executor = ThreadPoolExecutor(max_workers=1)
        self.last_activity = time.time()
        self.idle_timeout = self.config.get("worker", {}).get("idle_timeout", 300)
        self.lock = threading.Lock()
        self.running = True

    async def handle_client(self, reader: asyncio.StreamReader, writer: asyncio.StreamWriter):
        try:
            while self.running:
                raw_len = await reader.read(4)
                if len(raw_len) < 4:
                    break
                length = struct.unpack("<I", raw_len)[0]
                if length > 10_000_000:
                    break

                data = await reader.read(length)
                if len(data) < length:
                    break

                request = json.loads(data.decode("utf-8"))
                response = await self.process_request(request)
                self.last_activity = time.time()

                resp_bytes = json.dumps(response).encode("utf-8")
                writer.write(struct.pack("<I", len(resp_bytes)) + resp_bytes)
                await writer.drain()
        except asyncio.CancelledError:
            pass
        except (ConnectionResetError, BrokenPipeError, json.JSONDecodeError, struct.error) as e:
            print(f"[worker] Client handler error: {e}")
        finally:
            writer.close()
            await writer.wait_closed()

    def _extract_request(self, request: dict) -> tuple[str, dict]:
        """
        Normalize request format.
        Returns (request_type, payload_dict).
        Accepts:
          - {"type": "chat", "payload": {...}}
          - {"type": "WorkerRequest", "payload": {"request_type": "chat", ...}}
          - {"type": "health"}  (payload empty)
        """
        event_type = request.get("type", "")
        payload = request.get("payload", {}) or {}

        if event_type == "WorkerRequest":
            inner_type = payload.get("request_type") or payload.get("type", "unknown")
            return inner_type, payload
        else:
            return event_type, payload

    async def process_request(self, request: dict) -> dict:
        req_type, payload = self._extract_request(request)

        if req_type == "health":
            return await self._health()
        elif req_type == "chat":
            return await self._chat(payload)
        elif req_type == "generate":
            return await self._generate(payload)
        elif req_type == "unload":
            return await self._unload()
        elif req_type == "memory":
            return await self._memory()
        else:
            return _make_event(
                {
                    "type": "error",
                    "error": f"Unknown request type: {req_type}",
                }
            )

    async def _health(self) -> dict:
        return _make_event(
            {
                "type": "health",
                "status": "ok",
                "model_loaded": self.engine.is_loaded,
                "uptime": time.time() - getattr(self, "_start_time", time.time()),
            }
        )

    async def _chat(self, payload: dict) -> dict:
        messages = payload.get("messages", [])
        max_tokens = payload.get("max_tokens", 256)
        temperature = payload.get("temperature", 0.7)
        working_memory = payload.get("working_memory", []) or []

        if not messages:
            return _make_event(
                {
                    "type": "error",
                    "error": "Empty message/prompt",
                }
            )

        chat_messages = build_chat_messages(
            user_messages=messages,
            working_memory=working_memory,
            system_prompt=SYSTEM_PROMPT,
        )

        try:
            loop = asyncio.get_event_loop()
            result = await loop.run_in_executor(
                self.executor,
                lambda: self.engine.chat(
                    messages=chat_messages,
                    max_tokens=max_tokens,
                    temperature=temperature,
                ),
            )

            content = result.get("choices", [{}])[0].get("message", {}).get("content", "")
            finish_reason = result.get("choices", [{}])[0].get("finish_reason", "")
            usage = result.get("usage", {})

            return _make_event(
                {
                    "type": "response",
                    "result": {
                        "content": content,
                        "finish_reason": finish_reason,
                        "usage": usage,
                    },
                }
            )
        except (RuntimeError, OSError, ValueError) as e:
            return _make_event(
                {
                    "type": "error",
                    "error": f"Inference failed: {e!s}",
                }
            )

    async def _generate(self, payload: dict) -> dict:
        prompt = payload.get("prompt", "")
        max_tokens = payload.get("max_tokens", 256)
        temperature = payload.get("temperature", 0.7)

        if not prompt:
            return _make_event(
                {
                    "type": "error",
                    "error": "Empty message/prompt",
                }
            )

        try:
            loop = asyncio.get_event_loop()
            result = await loop.run_in_executor(
                self.executor,
                lambda: self.engine.generate(
                    prompt=prompt,
                    max_tokens=max_tokens,
                    temperature=temperature,
                ),
            )

            content = result.get("choices", [{}])[0].get("text", "")
            finish_reason = result.get("choices", [{}])[0].get("finish_reason", "")
            usage = result.get("usage", {})

            return _make_event(
                {
                    "type": "response",
                    "result": {
                        "content": content,
                        "finish_reason": finish_reason,
                        "usage": usage,
                    },
                }
            )
        except (RuntimeError, OSError, ValueError) as e:
            return _make_event(
                {
                    "type": "error",
                    "error": f"Inference failed: {e!s}",
                }
            )

    async def _unload(self) -> dict:
        self.engine.unload()
        gc.collect()
        return _make_event(
            {
                "type": "response",
                "result": {"status": "unloaded"},
            }
        )

    async def _memory(self) -> dict:
        mem = self.engine.get_memory_usage()
        return _make_event(
            {
                "type": "response",
                "result": mem,
            }
        )

    async def idle_monitor(self):
        while self.running:
            await asyncio.sleep(30)
            with self.lock:
                if self.engine.is_loaded and (time.time() - self.last_activity) > self.idle_timeout:
                    print("[worker] Idle timeout reached. Unloading model.")
                    self.engine.unload()
                    gc.collect()

    async def run(self):
        self._start_time = time.time()
        if os.path.exists(self.socket_path):
            os.remove(self.socket_path)

        server = await asyncio.start_unix_server(self.handle_client, path=self.socket_path)
        os.chmod(self.socket_path, 0o666)

        print(f"[worker] Listening on {self.socket_path}")

        idle_task = asyncio.create_task(self.idle_monitor())

        loop = asyncio.get_event_loop()
        for sig in (signal.SIGTERM, signal.SIGINT):
            loop.add_signal_handler(sig, lambda: asyncio.create_task(self.shutdown()))

        async with server:
            await server.serve_forever()

        idle_task.cancel()
        try:
            await idle_task
        except asyncio.CancelledError:
            pass

    async def shutdown(self):
        print("[worker] Shutting down...")
        self.running = False
        self.engine.unload()
        gc.collect()
        if os.path.exists(self.socket_path):
            os.remove(self.socket_path)


def main():
    server = WorkerServer()
    asyncio.run(server.run())


if __name__ == "__main__":
    main()
