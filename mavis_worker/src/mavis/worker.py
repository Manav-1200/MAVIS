import asyncio
import base64
import json
import os
import signal
import struct
import time
import uuid
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timezone

from mavis.core.config import load_config
from mavis.inference import SYSTEM_PROMPT, LlamaEngine, build_chat_messages
from mavis.stt.engine import STTEngine
from mavis.tts.engine import TTSEngine


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
        self.stt_engine = STTEngine(
            model_size="small",
            device="cpu",
            compute_type="int8",
            confidence_threshold=0.6,
        )
        self.tts_engine = TTSEngine()
        self.config = load_config()

        # Separate executors so STT never blocks behind TTS warm-up or LLM inference.
        self.llm_executor = ThreadPoolExecutor(max_workers=1, thread_name_prefix="llm")
        self.stt_executor = ThreadPoolExecutor(max_workers=1, thread_name_prefix="stt")
        self.tts_executor = ThreadPoolExecutor(max_workers=1, thread_name_prefix="tts")

        self.last_activity = time.time()
        self.idle_timeout = self.config.get("worker", {}).get("idle_timeout", 300)
        self.lock = asyncio.Lock()
        self.running = True

    async def handle_client(self, reader: asyncio.StreamReader, writer: asyncio.StreamWriter):
        try:
            while self.running:
                try:
                    raw_len = await reader.readexactly(4)
                except (asyncio.IncompleteReadError, BrokenPipeError, ConnectionResetError):
                    # Client closed connection normally (e.g., after reading response,
                    # or fire-and-forget warmup that now reads the response).
                    break

                length = struct.unpack("<I", raw_len)[0]
                if length > 10_000_000:
                    break

                data = await reader.readexactly(length)

                request = json.loads(data.decode("utf-8"))
                req_type, _ = self._extract_request(request)
                response = await self.process_request(request)

                # Only bump activity for actual work, not health pings
                if req_type not in ("health",):
                    self.last_activity = time.time()

                resp_bytes = json.dumps(response).encode("utf-8")
                try:
                    writer.write(struct.pack("<I", len(resp_bytes)) + resp_bytes)
                    await writer.drain()
                except (BrokenPipeError, ConnectionResetError, ConnectionAbortedError):
                    # Client closed connection before reading response
                    break
        except asyncio.CancelledError:
            raise
        except Exception:  # noqa: BLE001
            import traceback

            traceback.print_exc()
        finally:
            try:
                writer.close()
                await writer.wait_closed()
            except (BrokenPipeError, ConnectionResetError, ConnectionAbortedError):
                pass

    def _extract_request(self, request: dict) -> tuple[str, dict]:
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
        elif req_type == "stt":
            return await self._stt(payload)
        elif req_type == "tts":
            return await self._tts(payload)
        elif req_type == "warmup":
            return await self._warmup()
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
                "stt_loaded": self.stt_engine.is_loaded(),
                "tts_loaded": self.tts_engine.is_loaded,
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
                self.llm_executor,
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
                self.llm_executor,
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

    async def _stt(self, payload: dict) -> dict:
        audio_b64 = payload.get("audio", "")
        if not audio_b64:
            return _make_event(
                {
                    "type": "error",
                    "error": "Missing audio field",
                }
            )

        try:
            audio_bytes = base64.b64decode(audio_b64)
            print(f"[worker] STT request: {len(audio_bytes)} bytes", flush=True)

            # One-shot active listen override: if MAVIS_ACTIVE_LISTEN=1, bypass
            # the confidence gate for this utterance and clear the flag.
            active_listen = os.environ.pop("MAVIS_ACTIVE_LISTEN", None) == "1"
            if active_listen:
                print("[worker] Active listen override enabled for this utterance", flush=True)

            loop = asyncio.get_event_loop()
            text = await loop.run_in_executor(
                self.stt_executor,
                lambda: self.stt_engine.transcribe(
                    audio_bytes,
                    sample_rate=16000,
                    bypass_confidence=active_listen,
                ),
            )
            print(f"[worker] STT result: '{text[:80]}...'", flush=True)
            return _make_event(
                {
                    "type": "response",
                    "result": {"text": text},
                }
            )
        except (RuntimeError, OSError, ValueError) as e:
            return _make_event(
                {
                    "type": "error",
                    "error": f"STT failed: {e!s}",
                }
            )

    async def _tts(self, payload: dict) -> dict:
        text = payload.get("text", "")
        voice = payload.get("voice")
        speed = payload.get("speed", 1.0)

        if not text:
            return _make_event(
                {
                    "type": "error",
                    "error": "Missing text field",
                }
            )

        try:
            print(f"[worker] TTS request: '{text[:60]}...'", flush=True)
            loop = asyncio.get_event_loop()
            audio_b64 = await loop.run_in_executor(
                self.tts_executor,
                lambda: self.tts_engine.synthesize(text, voice=voice, speed=speed),
            )
            return _make_event(
                {
                    "type": "response",
                    "result": {"audio": audio_b64, "format": "wav"},
                }
            )
        except (RuntimeError, OSError, ValueError) as e:
            return _make_event(
                {
                    "type": "error",
                    "error": f"TTS failed: {e!s}",
                }
            )

    async def _warmup(self) -> dict:
        """Eagerly load the LLM so the next chat request doesn't block."""
        print("[worker] LLM warm-up requested", flush=True)
        loop = asyncio.get_event_loop()
        await loop.run_in_executor(
            self.llm_executor,
            self.engine.warm_up,
        )
        print("[worker] LLM warm-up complete", flush=True)
        return _make_event(
            {
                "type": "response",
                "result": {"status": "warmed_up"},
            }
        )

    async def _unload(self) -> dict:
        self.engine.unload()
        self.stt_engine.unload()
        self.tts_engine.unload()
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
        print(f"[worker] Idle monitor started (timeout={self.idle_timeout}s)", flush=True)
        while self.running:
            await asyncio.sleep(30)
            async with self.lock:
                now = time.time()
                # STT model stays permanently loaded (Phase 5 latency fix).
                # Only LLM and TTS are eligible for idle unload.
                llm_idle = now - self.last_activity > self.idle_timeout
                tts_idle = now - self.tts_engine.last_activity > self.idle_timeout

                any_loaded = self.engine.is_loaded or self.tts_engine.is_loaded
                all_idle = llm_idle and tts_idle

                if any_loaded and all_idle:
                    print(
                        "[worker] Idle timeout reached. Unloading models (STT stays resident).",
                        flush=True,
                    )
                    self.engine.unload()
                    # Intentionally do NOT unload STT
                    self.tts_engine.unload()
                elif any_loaded:
                    print(
                        f"[worker] Idle check: loaded, llm_idle={llm_idle}, tts_idle={tts_idle}",
                        flush=True,
                    )

    async def run(self):
        self._start_time = time.time()
        if os.path.exists(self.socket_path):
            os.remove(self.socket_path)

        server = await asyncio.start_unix_server(self.handle_client, path=self.socket_path)
        os.chmod(self.socket_path, 0o666)

        print(f"[worker] Listening on {self.socket_path}", flush=True)

        idle_task = asyncio.create_task(self.idle_monitor())

        # Warm up AI subsystems in the background so first requests are fast.
        # STT model download (~500MB) can take 2-5 minutes on first run.
        loop = asyncio.get_event_loop()
        loop.run_in_executor(self.tts_executor, self.tts_engine.warm_up)
        loop.run_in_executor(self.stt_executor, self.stt_engine.warm_up)

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
        print("[worker] Shutting down...", flush=True)
        self.running = False
        self.engine.unload()
        self.stt_engine.unload()
        self.tts_engine.unload()
        self.llm_executor.shutdown(wait=False)
        self.stt_executor.shutdown(wait=False)
        self.tts_executor.shutdown(wait=False)
        if os.path.exists(self.socket_path):
            os.remove(self.socket_path)


def main():
    server = WorkerServer()
    asyncio.run(server.run())


if __name__ == "__main__":
    main()
