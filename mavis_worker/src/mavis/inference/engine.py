"""Local LLM inference engine using llama-cpp-python."""

import gc
import logging
import time
from pathlib import Path
from typing import Any

logger = logging.getLogger(__name__)


class LlamaEngine:
    """Manages model loading and inference."""

    def __init__(self, config: dict[str, Any]):
        self.config = config
        self.model_path: str | None = None
        self._llm = None
        self._loaded = False
        self._load_time = 0.0

    def _resolve_model_path(self) -> str:
        """Resolve model path from config or well-known locations."""
        model_path = self.config.get("model", {}).get("path")
        if model_path:
            expanded = Path(model_path).expanduser()
            if expanded.exists():
                return str(expanded)

        candidates = [
            Path.home() / ".local/share/mavis/models/Llama-3.1-8B-Instruct-Q4_K_M.gguf",
            Path.home() / ".local/share/mavis/models/Phi-3-mini-4k-instruct-Q4_K_M.gguf",
        ]
        for candidate in candidates:
            if candidate.exists():
                return str(candidate)

        raise FileNotFoundError("No GGUF model found. Download one to ~/.local/share/mavis/models/")

    def load_model(self) -> None:
        """Lazy-load the model. Idempotent."""
        if self._loaded:
            return

        from llama_cpp import Llama

        self.model_path = self._resolve_model_path()
        logger.info(f"Loading model from {self.model_path}")

        n_ctx = self.config.get("model", {}).get("n_ctx", 4096)
        n_gpu_layers = self.config.get("model", {}).get("n_gpu_layers", -1)
        verbose = self.config.get("model", {}).get("verbose", False)

        start = time.time()
        self._llm = Llama(
            model_path=self.model_path,
            n_ctx=n_ctx,
            n_gpu_layers=n_gpu_layers,
            verbose=verbose,
        )
        self._load_time = time.time() - start
        self._loaded = True
        logger.info(f"Model loaded in {self._load_time:.2f}s")

    def unload(self) -> None:
        """Unload model and free VRAM."""
        if not self._loaded:
            return
        logger.info("Unloading model...")
        del self._llm
        self._llm = None
        self._loaded = False
        gc.collect()
        logger.info("Model unloaded.")

    @property
    def is_loaded(self) -> bool:
        return self._loaded

    def get_memory_usage(self) -> dict[str, Any]:
        """Return VRAM usage if available."""
        try:
            from llama_cpp import cuda_get_device_memory

            free, total = cuda_get_device_memory()
            used = total - free
            return {
                "gpu_total_mb": round(total / 1024**2, 1),
                "gpu_used_mb": round(used / 1024**2, 1),
            }
        except ImportError:
            return {"gpu_total_mb": 0, "gpu_used_mb": 0}

    def generate(
        self,
        prompt: str,
        max_tokens: int = 512,
        temperature: float = 0.7,
        stop: list[str] | None = None,
    ) -> dict[str, Any]:
        """Raw completion. Loads model if needed."""
        self.load_model()
        stop = stop or ["<|eot_id|>", "<|endoftext|>"]

        start = time.time()
        output = self._llm(
            prompt,
            max_tokens=max_tokens,
            temperature=temperature,
            stop=stop,
        )
        inference_time = time.time() - start

        text = output["choices"][0]["text"]
        tokens = output["usage"]["completion_tokens"]

        return {
            "text": text,
            "tokens": tokens,
            "inference_time": round(inference_time, 3),
            "tokens_per_sec": round(tokens / inference_time, 2) if inference_time > 0 else 0,
        }

    def chat(
        self,
        messages: list[dict[str, str]],
        max_tokens: int = 512,
        temperature: float = 0.7,
    ) -> dict[str, Any]:
        """Chat completion using the model's built-in chat template."""
        self.load_model()

        start = time.time()
        output = self._llm.create_chat_completion(
            messages=messages,
            max_tokens=max_tokens,
            temperature=temperature,
        )
        inference_time = time.time() - start

        text = output["choices"][0]["message"]["content"]
        tokens = output["usage"]["completion_tokens"]

        return {
            "text": text,
            "tokens": tokens,
            "inference_time": round(inference_time, 3),
            "tokens_per_sec": round(tokens / inference_time, 2) if inference_time > 0 else 0,
        }
