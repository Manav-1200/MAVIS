import gc
import re
import subprocess
import time
from pathlib import Path
from typing import Any

try:
    from llama_cpp import Llama
except ImportError as e:
    raise ImportError(
        "llama-cpp-python not installed. Run:\n"
        " CMAKE_ARGS='-DGGML_CUDA=on' pip install llama-cpp-python --force-reinstall --no-cache-dir"
    ) from e


class LlamaEngine:
    def __init__(self, model_path: str | None = None, n_gpu_layers: int = 20):
        self._model_path = model_path
        self._n_gpu_layers = n_gpu_layers
        self._llm: Llama | None = None
        self._model_name_hint: str = ""

    def _resolve_model_path(self) -> str:
        if self._model_path:
            configured = Path(self._model_path).expanduser()
            if configured.exists():
                return str(configured)

        candidates = [
            Path.home() / ".local/share/mavis/models/Phi-3-mini-4k-instruct-Q4_K_M.gguf",
            Path.home() / ".local/share/mavis/models/tinyllama-1.1b-chat-v1.0.Q4_K_M.gguf",
        ]

        for candidate in candidates:
            if candidate.exists():
                return str(candidate)

        raise FileNotFoundError("No GGUF model found. Download one to ~/.local/share/mavis/models/")

    def _detect_model_type(self, path: str) -> str:
        p = path.lower()
        if "tinyllama" in p:
            return "tinyllama"
        if "phi-3" in p or "phi3" in p:
            return "phi3"
        if "llama-3" in p or "llama3" in p:
            return "llama3"
        return "unknown"

    def load_model(self):
        if self._llm is not None:
            return

        path = self._resolve_model_path()
        self._model_name_hint = self._detect_model_type(path)
        print(f"[engine] Loading model: {path} (type={self._model_name_hint})")

        self._llm = Llama(
            model_path=path,
            n_ctx=4096,
            n_gpu_layers=self._n_gpu_layers,
            verbose=False,
        )
        print("[engine] Model loaded.")

    def unload(self):
        if self._llm is not None:
            print("[engine] Unloading model...")
            if hasattr(self._llm, "cache"):
                self._llm.cache = None
            if hasattr(self._llm, "_cache"):
                self._llm._cache = None
            del self._llm
            self._llm = None
            for _ in range(3):
                gc.collect()
            try:
                import torch

                torch.cuda.empty_cache()
                print("[engine] CUDA cache cleared.")
            except ImportError:
                pass
            print("[engine] Model unloaded.")

    @property
    def is_loaded(self) -> bool:
        return self._llm is not None

    def warm_up(self, system_prompt: str | None = None):
        """
        Eagerly load model weights so the next chat request is fast.

        With a system prompt, also evaluate it now. Warm-up is requested the
        moment the user starts speaking, so this runs while they talk; every
        prompt starts with the same system block, and llama.cpp reuses an
        evaluated prefix instead of recomputing it. Best effort — a failure
        here only means the chat request does the work itself.
        """
        self.load_model()
        if not system_prompt:
            return
        prefix = self._format_chat_prompt([{"role": "system", "content": system_prompt}])
        if prefix is None:
            return
        # Drop the trailing "<|assistant|>" opener the formatter appends:
        # the real prompt continues with working memory there instead.
        prefix = prefix.rsplit("<|assistant|>", 1)[0]
        try:
            started = time.perf_counter()
            self._llm.create_completion(prompt=prefix, max_tokens=1, temperature=0.0)
            print(
                f"[engine] System prompt prefilled in {time.perf_counter() - started:.2f}s",
                flush=True,
            )
        except (RuntimeError, ValueError) as e:
            print(f"[engine] Prefill skipped: {e}", flush=True)

    def get_memory_usage(self) -> dict[str, float]:
        if not self.is_loaded:
            return {"gpu_total_mb": 0.0, "gpu_used_mb": 0.0}

        try:
            result = subprocess.run(
                [
                    "nvidia-smi",
                    "--query-gpu=memory.total,memory.used",
                    "--format=csv,noheader,nounits",
                ],
                capture_output=True,
                text=True,
                check=True,
                timeout=5,
            )
            total_str, used_str = result.stdout.strip().split(", ")
            return {
                "gpu_total_mb": float(total_str),
                "gpu_used_mb": float(used_str),
            }
        except (subprocess.SubprocessError, OSError, ValueError) as e:
            print(f"[engine] GPU memory query failed: {e}")
            return {"gpu_total_mb": 0.0, "gpu_used_mb": 0.0}

    def _format_chat_prompt(self, messages: list[dict[str, str]]) -> str | None:
        """Manually format chat prompt for models with known templates."""
        if self._model_name_hint == "tinyllama":
            parts: list[str] = []
            for msg in messages:
                role = msg.get("role", "user")
                content = msg.get("content", "")
                if role == "system":
                    parts.append(f"<|system|>\n{content}")
                elif role == "user":
                    parts.append(f"<|user|>\n{content}")
                elif role == "assistant":
                    parts.append(f"<|assistant|>\n{content}")
            parts.append("<|assistant|>\n")
            return "\n".join(parts)

        if self._model_name_hint == "phi3":
            parts: list[str] = []
            for msg in messages:
                role = msg.get("role", "user")
                content = msg.get("content", "")
                if role == "system":
                    parts.append(f"<|system|>\n{content}<|end|>")
                elif role == "user":
                    parts.append(f"<|user|>\n{content}<|end|>")
                elif role == "assistant":
                    parts.append(f"<|assistant|>\n{content}<|end|>")
            parts.append("<|assistant|>\n")
            return "\n".join(parts)

        return None

    def _get_stop_tokens(self) -> list[str]:
        """Return model-specific stop tokens to prevent runaway generation."""
        if self._model_name_hint == "tinyllama":
            return ["<|user|>", "<|system|>", "<|assistant|>", "</s>"]
        if self._model_name_hint == "phi3":
            return [
                "<|end|>",
                "<|user|>",
                "<|system|>",
                "<|assistant|>",
                "<|endoftext|>",
                "</s>",
            ]
        return []

    def _post_process(self, text: str) -> str:
        """Aggressively clean generation: cut stops, split separators, truncate."""
        if not text:
            return "I'm here."

        # 1. Cut at first stop token
        for stop in self._get_stop_tokens():
            if stop in text:
                text = text[: text.index(stop)]

        # A leading newline used to make the "first line" below empty.
        text = text.lstrip()

        # 2. Split on structural separators and keep only the first segment
        for sep in ["===", "---", "***", "___", "\n\n", "\n"]:
            if sep in text:
                text = text.split(sep, 1)[0]

        # 3. Strip markdown artifacts (bullets, numbering, headers, links, code)
        text = re.sub(r"^[-*•]\s+", "", text, flags=re.MULTILINE)
        text = re.sub(r"^\d+\.\s+", "", text, flags=re.MULTILINE)
        text = re.sub(r"^#+\s+", "", text, flags=re.MULTILINE)
        text = re.sub(r"\[.*?\]\(.*?\)", "", text)
        text = re.sub(r"`.*?`", "", text)

        # 4. Clean whitespace
        text = re.sub(r"\s+", " ", text).strip()

        # 5. Truncate to first 1-2 sentences
        sentences = re.split(r"(?<=[.!?])\s+", text)
        if len(sentences) > 2:
            text = " ".join(sentences[:2])

        # 6. Hard cap at 180 chars, ending at a sentence boundary if possible
        if len(text) > 180:
            match = re.search(r".{1,180}[.!?]", text)
            text = match.group(0) if match else text[:180]

        return text.strip() or "I'm here."

    def generate(
        self,
        prompt: str,
        max_tokens: int = 256,
        temperature: float = 0.7,
        stop: list[str] | None = None,
    ) -> dict[str, Any]:
        self.load_model()
        return self._llm(
            prompt=prompt,
            max_tokens=max_tokens,
            temperature=temperature,
            stop=stop or [],
        )

    @staticmethod
    def _reply_complete(text: str) -> bool:
        """
        True once generation has produced everything _post_process keeps.

        _post_process keeps the first line, at most two sentences and at
        most 180 characters. Everything generated past that point was
        thrown away — with max_tokens=256 that was most of an 11-second
        wait in the 2026-09-22 log. Stopping here gives the same reply,
        sooner. Mirrors _post_process's own rules, so it never stops before
        something that would have been kept.
        """
        t = text.lstrip()
        if not t:
            return False
        if "\n" in t:
            return True
        if any(sep in t for sep in ("===", "---", "***", "___")):
            return True
        if len(t) > 200:
            return True
        # A sentence boundary is punctuation followed by whitespace — the
        # same rule _post_process splits on, so "3.5" or a trailing "." at
        # the very end (not yet followed by anything) don't count.
        return len(re.findall(r"[.!?]+[\"')\]]*\s", t)) >= 2

    def _stream(self, chunks, get_text) -> tuple[str, str, int, float | None]:
        """Consume a streaming completion, stopping as soon as the reply is complete."""
        started = time.perf_counter()
        first_token_at: float | None = None
        text = ""
        finish_reason = ""
        tokens = 0
        for chunk in chunks:
            choice = chunk.get("choices", [{}])[0]
            piece = get_text(choice) or ""
            if piece and first_token_at is None:
                first_token_at = time.perf_counter() - started
            text += piece
            tokens += 1
            finish_reason = choice.get("finish_reason") or finish_reason
            if self._reply_complete(text):
                finish_reason = "complete"
                break
        return text, finish_reason, tokens, first_token_at

    def chat(
        self,
        messages: list[dict[str, str]],
        max_tokens: int = 256,
        temperature: float = 0.7,
    ) -> dict[str, Any]:
        self.load_model()
        started = time.perf_counter()

        manual_prompt = self._format_chat_prompt(messages)
        if manual_prompt is not None:
            print(f"[engine] Prompt ({self._model_name_hint}):\n{manual_prompt}\n")
            chunks = self._llm(
                prompt=manual_prompt,
                max_tokens=max_tokens,
                temperature=temperature,
                stop=self._get_stop_tokens(),
                stream=True,
            )
            raw_text, finish_reason, tokens, ttft = self._stream(
                chunks, lambda c: c.get("text", "")
            )
        else:
            # Native path — llama.cpp uses the GGUF's own chat template.
            chunks = self._llm.create_chat_completion(
                messages=messages,
                max_tokens=max_tokens,
                temperature=temperature,
                stream=True,
            )
            raw_text, finish_reason, tokens, ttft = self._stream(
                chunks, lambda c: c.get("delta", {}).get("content", "")
            )

        elapsed = time.perf_counter() - started
        # The one number that says where a slow reply went: time to first
        # token is prompt processing, the rest is generation.
        print(
            f"[engine] Reply: {tokens} tokens in {elapsed:.2f}s "
            f"(first token {ttft if ttft is not None else elapsed:.2f}s, finish={finish_reason})",
            flush=True,
        )

        # Fix: this used to only run for tinyllama/phi3 (manual_prompt branch),
        # so llama3/unknown models skipped style cleanup entirely.
        text = self._post_process(raw_text)

        return {
            "choices": [
                {
                    "message": {"content": text, "role": "assistant"},
                    "finish_reason": finish_reason,
                }
            ],
            "usage": {"completion_tokens": tokens, "elapsed_s": round(elapsed, 3)},
        }
