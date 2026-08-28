import gc
import re
import subprocess
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
            return self._model_path

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

    def warm_up(self):
        """Eagerly load model weights so the next chat request is fast."""
        self.load_model()

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

    def chat(
        self,
        messages: list[dict[str, str]],
        max_tokens: int = 256,
        temperature: float = 0.7,
    ) -> dict[str, Any]:
        self.load_model()

        manual_prompt = self._format_chat_prompt(messages)
        if manual_prompt is not None:
            print(f"[engine] Prompt ({self._model_name_hint}):\n{manual_prompt}\n")
            result = self._llm(
                prompt=manual_prompt,
                max_tokens=max_tokens,
                temperature=temperature,
                stop=self._get_stop_tokens(),
            )
            raw_text = result.get("choices", [{}])[0].get("text", "")
            text = self._post_process(raw_text)

            # Safety fallback if the model emitted nothing but stop tokens
            if not text:
                text = "I'm here."

            return {
                "choices": [
                    {
                        "message": {"content": text, "role": "assistant"},
                        "finish_reason": result.get("choices", [{}])[0].get("finish_reason", ""),
                    }
                ],
                "usage": result.get("usage", {}),
            }

        return self._llm.create_chat_completion(
            messages=messages,
            max_tokens=max_tokens,
            temperature=temperature,
        )
