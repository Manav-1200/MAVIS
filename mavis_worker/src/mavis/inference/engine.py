import gc
import subprocess
from pathlib import Path
from typing import Any

try:
    from llama_cpp import Llama
except ImportError as e:
    raise ImportError(
        "llama-cpp-python not installed. Run:\n"
        "  CMAKE_ARGS='-DGGML_CUDA=on' pip install llama-cpp-python --force-reinstall --no-cache-dir"
    ) from e


class LlamaEngine:
    def __init__(self, model_path: str | None = None, n_gpu_layers: int = -1):
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
            del self._llm
            self._llm = None
            gc.collect()

    @property
    def is_loaded(self) -> bool:
        return self._llm is not None

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
        """
        Manually format chat prompt for models whose GGUF lacks proper chat template.
        Returns None if native create_chat_completion should be used.
        """
        if self._model_name_hint == "tinyllama":
            parts = []
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

        # For Phi-3, Llama-3, and other modern models, let llama-cpp handle it
        return None

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
            result = self._llm(
                prompt=manual_prompt,
                max_tokens=max_tokens,
                temperature=temperature,
                stop=["<|user|>", "<|system|>", "<|assistant|>"],
            )
            text = result.get("choices", [{}])[0].get("text", "")
            return {
                "choices": [
                    {
                        "message": {"content": text, "role": "assistant"},
                        "finish_reason": result.get("choices", [{}])[0].get("finish_reason", ""),
                    }
                ],
                "usage": result.get("usage", {}),
            }

        # Native chat completion for models with proper templates
        return self._llm.create_chat_completion(
            messages=messages,
            max_tokens=max_tokens,
            temperature=temperature,
        )
