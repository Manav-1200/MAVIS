# mavis_worker/src/mavis/inference/engine.py
import gc
import json
import os
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


# How much of a reply is kept. Two sentences clipped replies; speech starts
# with the first sentence, so a third costs no waiting.
MAX_SENTENCES = 3
MAX_CHARS = 320


# The most context each model was trained for; more than that only confuses it.
TRAINED_CONTEXT = {"phi3": 4096, "tinyllama": 2048}

# Fallback layers on the graphics card for a model other than the one
# configured, which a full offload might not fit.
FALLBACK_GPU_LAYERS = 20

# Room kept free in the context beyond the reply.
CONTEXT_MARGIN = 64

# Tokens looked back over for repeats, and how much a repeat is held back:
# enough to reach its own last reply, so it says something new.
REPEAT_WINDOW = 256
REPEAT_PENALTY = 1.1


class LlamaEngine:
    def __init__(self, model_path: str | None = None, n_gpu_layers: int = 20, n_ctx: int = 4096):
        self._model_path = model_path
        self._n_gpu_layers = n_gpu_layers
        self._n_ctx = n_ctx
        self._llm: Llama | None = None
        self._model_name_hint: str = ""

    def _resolve_model_path(self) -> str:
        if self._model_path:
            configured = Path(self._model_path).expanduser()
            if configured.exists():
                return str(configured)

        self._n_gpu_layers = (
            min(self._n_gpu_layers, FALLBACK_GPU_LAYERS)
            if self._n_gpu_layers >= 0
            else FALLBACK_GPU_LAYERS
        )
        candidates = [
            Path.home() / ".local/share/mavis/models/qwen2.5-3b-instruct-q4_k_m.gguf",
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
        if "qwen" in p:
            return "chatml"
        return "unknown"

    def load_model(self):
        if self._llm is not None:
            return

        path = self._resolve_model_path()
        self._model_name_hint = self._detect_model_type(path)
        self._n_ctx = min(self._n_ctx, TRAINED_CONTEXT.get(self._model_name_hint, self._n_ctx))
        print(
            f"[engine] Loading model: {path} (type={self._model_name_hint}, context={self._n_ctx})"
        )

        try:
            self._llm = Llama(
                model_path=path,
                n_ctx=self._n_ctx,
                n_gpu_layers=self._n_gpu_layers,
                last_n_tokens_size=REPEAT_WINDOW,
                verbose=False,
            )
        except ValueError:
            if self._n_gpu_layers < 0 or self._n_gpu_layers > FALLBACK_GPU_LAYERS:
                # Didn't fit on the graphics card whole: keep part on the processor.
                print(
                    f"[engine] Loading failed; retrying with {FALLBACK_GPU_LAYERS} layers on the GPU"
                )
                self._n_gpu_layers = FALLBACK_GPU_LAYERS
                self._llm = Llama(
                    model_path=path,
                    n_ctx=self._n_ctx,
                    n_gpu_layers=self._n_gpu_layers,
                    last_n_tokens_size=REPEAT_WINDOW,
                    verbose=False,
                )
            else:
                raise
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
        # Drop the assistant opener the formatter appends: the real prompt
        # continues with working memory there instead.
        for opener in ("<|im_start|>assistant", "<|assistant|>"):
            if opener in prefix:
                prefix = prefix.rsplit(opener, 1)[0]
                break
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

        if self._model_name_hint == "chatml":
            parts = [
                f"<|im_start|>{m.get('role', 'user')}\n{m.get('content', '')}<|im_end|>"
                for m in messages
            ]
            parts.append("<|im_start|>assistant\n")
            return "\n".join(parts)

        return None

    @staticmethod
    def _read_call(call: str) -> str | None:
        """'<tool_call>{"name": "volume_set", "arguments": {"level": 40}}</tool_call>'
        -> "volume_set 40"."""
        body = call.removeprefix("<tool_call>").split("</tool_call>")[0].strip()
        try:
            parsed = json.loads(body)
        except json.JSONDecodeError:
            return None
        args = [str(v) for v in (parsed.get("arguments") or {}).values()]
        return " ".join([str(parsed.get("name", "")), *args]).strip() or None

    def _tokens(self, messages: list[dict[str, str]]) -> int:
        prompt = self._format_chat_prompt(messages)
        if prompt is None:
            prompt = "\n".join(m.get("content", "") for m in messages)
        return len(self._llm.tokenize(prompt.encode("utf-8"), add_bos=False, special=True))

    def _fit(self, messages: list[dict[str, str]], max_tokens: int) -> list[dict[str, str]]:
        """Drop the oldest turns of the conversation until the prompt fits the
        context with room for the reply. The system prompt and the current
        message always stay."""
        budget = self._n_ctx - max_tokens - CONTEXT_MARGIN
        fitted = list(messages)
        dropped = 0
        while len(fitted) > 2 and self._tokens(fitted) > budget:
            del fitted[1]
            dropped += 1
            # The conversation still starts with the user.
            while len(fitted) > 2 and fitted[1].get("role") == "assistant":
                del fitted[1]
                dropped += 1
        if dropped:
            print(f"[engine] Context: {dropped} oldest turn(s) left out to fit", flush=True)
        return fitted

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
        if self._model_name_hint == "chatml":
            return ["<|im_end|>", "<|im_start|>", "<|endoftext|>"]
        return []

    def _post_process(self, text: str) -> str:
        """Aggressively clean generation: cut stops, split separators, truncate."""
        if not text:
            return ""

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

        # 5. Narration about MAVIS keeps only what she says:
        # 'Mavis says: "..."' → '...'
        # Only when the reply opens with her name, so "I'm Mavis, and the
        # song is "Sunbeam"" keeps all of it.
        narrated = re.match(r"mavis\b[^\"“]*[\"“](.+?)[\"”]", text, flags=re.IGNORECASE)
        if narrated:
            text = narrated.group(1).strip()

        # Working-memory labels the model copied: "[mavis] You are looking at…"
        text = re.sub(r"^\[(?:mavis|user|[a-z_]+)\]\s*", "", text, flags=re.IGNORECASE)

        # 6. Truncate to the first few sentences.
        sentences = re.split(r"(?<=[.!?])\s+", text)
        text = " ".join(sentences)
        if len(sentences) > MAX_SENTENCES:
            text = " ".join(sentences[:MAX_SENTENCES])

        # 7. Hard cap, ending at a sentence boundary if possible
        if len(text) > MAX_CHARS:
            match = re.search(rf".{{1,{MAX_CHARS}}}[.!?]", text)
            text = match.group(0) if match else text[:MAX_CHARS]

        return text.strip()

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

        _post_process keeps the first line, at most MAX_SENTENCES sentences
        and at most MAX_CHARS characters. Everything generated past that point was
        thrown away — with max_tokens=256 that could be most of the wait.
        Stopping here gives the same reply,
        sooner. Mirrors _post_process's own rules, so it never stops before
        something that would have been kept.
        """
        t = text.lstrip()
        # A tool call runs to its closing tag, which the grammar ends on.
        if not t or t.startswith("<"):
            return False
        if "\n" in t:
            return True
        if any(sep in t for sep in ("===", "---", "***", "___")):
            return True
        if len(t) > MAX_CHARS + 20:
            return True
        # A sentence boundary is punctuation followed by whitespace — the
        # same rule _post_process splits on, so "3.5" or a trailing "." at
        # the very end (not yet followed by anything) don't count.
        return len(re.findall(r"[.!?]+[\"')\]]*\s", t)) >= MAX_SENTENCES

    def _stream(self, chunks, get_text, early: bool = True) -> tuple[str, str, int, float | None]:
        """Consume a streaming completion, stopping as soon as the reply is
        complete; with `early` off, to the end (notes, not speech)."""
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
            if early and self._reply_complete(text):
                finish_reason = "complete"
                break
        return text, finish_reason, tokens, first_token_at

    def _grammar(self, text: str):
        """Compiled once per grammar text; the action list rarely changes."""
        from llama_cpp import LlamaGrammar

        if getattr(self, "_grammar_text", None) != text:
            self._grammar_text = text
            self._grammar_compiled = LlamaGrammar.from_string(text, verbose=False)
        return self._grammar_compiled

    def chat(
        self,
        messages: list[dict[str, str]],
        max_tokens: int = 256,
        temperature: float = 0.7,
        grammar: str | None = None,
        raw: bool = False,
    ) -> dict[str, Any]:
        """A reply, or a tool call. A call comes back as "action" (the
        action's name and argument, as the core reads them) and "call" (as
        the model wrote it). `raw` keeps the text whole, for notes rather
        than speech."""
        self.load_model()
        started = time.perf_counter()
        extra = {"grammar": self._grammar(grammar)} if grammar else {}
        messages = self._fit(messages, max_tokens)

        manual_prompt = self._format_chat_prompt(messages)
        if manual_prompt is not None:
            # The whole prompt is ~70 lines a turn; only when asked for.
            if os.environ.get("MAVIS_PROMPT_DEBUG") == "1":
                print(f"[engine] Prompt ({self._model_name_hint}):\n{manual_prompt}\n")
            chunks = self._llm(
                prompt=manual_prompt,
                max_tokens=max_tokens,
                temperature=temperature,
                repeat_penalty=REPEAT_PENALTY,
                stop=self._get_stop_tokens(),
                stream=True,
                **extra,
            )
            raw_text, finish_reason, tokens, ttft = self._stream(
                chunks, lambda c: c.get("text", ""), early=not raw
            )
        else:
            # Native path — llama.cpp uses the GGUF's own chat template.
            chunks = self._llm.create_chat_completion(
                messages=messages,
                max_tokens=max_tokens,
                temperature=temperature,
                repeat_penalty=REPEAT_PENALTY,
                stream=True,
                **extra,
            )
            raw_text, finish_reason, tokens, ttft = self._stream(
                chunks, lambda c: c.get("delta", {}).get("content", ""), early=not raw
            )

        elapsed = time.perf_counter() - started
        # The one number that says where a slow reply went: time to first
        # token is prompt processing, the rest is generation.
        print(
            f"[engine] Reply: {tokens} tokens in {elapsed:.2f}s "
            f"(first token {ttft if ttft is not None else elapsed:.2f}s, finish={finish_reason})",
            flush=True,
        )

        action, call, unreadable = None, "", False
        if raw_text.strip().startswith("<"):
            call = raw_text.strip()
            action = self._read_call(call)
            # A call that can't be read is never spoken.
            if action is None:
                print(f"[engine] Unreadable tool call: {call[:120]!r}", flush=True)
                call, unreadable = "", True
        if raw:
            for stop in self._get_stop_tokens():
                raw_text = raw_text.split(stop, 1)[0]
            text = raw_text.strip()
        else:
            text = "" if action or unreadable else self._post_process(raw_text)

        return {
            "action": action,
            "call": call,
            "choices": [
                {
                    "message": {"content": text, "role": "assistant"},
                    "finish_reason": finish_reason,
                }
            ],
            "usage": {"completion_tokens": tokens, "elapsed_s": round(elapsed, 3)},
        }
