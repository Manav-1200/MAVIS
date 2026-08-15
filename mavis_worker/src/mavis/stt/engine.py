"""
MAVIS STT Engine — wraps faster-whisper for local speech-to-text.
Lazy-loads the model and supports idle unload to reclaim VRAM.
"""

from __future__ import annotations

import gc
import logging
import time

import numpy as np

logger = logging.getLogger("mavis.stt")


class STTEngine:
    """
    AI-agnostic STT wrapper. Currently backed by faster-whisper.

    Defaults to CPU / int8 to avoid competing with the LLM for VRAM.
    If you have headroom (e.g. Phi-3 unloaded), set device="cuda"
    and compute_type="float16" in worker config.
    """

    def __init__(
        self,
        model_size: str = "base",
        device: str = "cpu",
        compute_type: str = "int8",
    ) -> None:
        self.model_size = model_size
        self.device = device
        self.compute_type = compute_type
        self._model: object | None = None
        self._last_activity = time.time()

    # --------------------------------------------------------------------- #
    # Lifecycle
    # --------------------------------------------------------------------- #
    def _load(self) -> None:
        """Lazy-load the faster-whisper model."""
        if self._model is not None:
            return

        try:
            from faster_whisper import WhisperModel
        except ImportError as exc:
            raise RuntimeError(
                "faster-whisper is not installed. Run: pip install faster-whisper"
            ) from exc

        logger.info(
            "Loading STT model: size=%s device=%s compute_type=%s",
            self.model_size,
            self.device,
            self.compute_type,
        )
        self._model = WhisperModel(
            self.model_size,
            device=self.device,
            compute_type=self.compute_type,
        )
        logger.info("STT model loaded.")

    def unload(self) -> None:
        """Aggressively unload the model and free GPU memory."""
        if self._model is None:
            return

        logger.info("Unloading STT model...")
        del self._model
        self._model = None
        gc.collect()

        # Best-effort CUDA cache clear
        try:
            import torch

            torch.cuda.empty_cache()
            logger.info("STT CUDA cache cleared.")
        except Exception:  # noqa: BLE001, S110
            pass

        logger.info("STT model unloaded.")

    # --------------------------------------------------------------------- #
    # Inference
    # --------------------------------------------------------------------- #
    def transcribe(self, audio_bytes: bytes, sample_rate: int = 16000) -> str:
        """
        Transcribe raw PCM audio (float32, mono, 16 kHz).

        Args:
            audio_bytes: Raw float32 little-endian PCM.
            sample_rate: Expected sample rate (must match audio data).

        Returns:
            Transcribed text, stripped and normalized.
        """
        self._load()
        self._last_activity = time.time()

        audio = np.frombuffer(audio_bytes, dtype=np.float32)
        if audio.size == 0:
            return ""

        # Diagnostics: log what the model actually receives
        logger.info(
            "STT input: samples=%d duration=%.2fs min=%.4f max=%.4f mean=%.4f",
            audio.size,
            audio.size / sample_rate,
            float(audio.min()),
            float(audio.max()),
            float(audio.mean()),
        )

        segments, info = self._model.transcribe(
            audio,
            beam_size=5,
            language="en",
            condition_on_previous_text=False,
            vad_filter=False,  # Rust VAD already segmented; don't double-filter
        )

        text = " ".join(seg.text for seg in segments).strip()
        logger.info(
            "Transcribed (%s, prob=%.2f): %s",
            info.language,
            info.language_probability,
            text[:120],
        )
        return text

    # --------------------------------------------------------------------- #
    # Idle monitoring
    # --------------------------------------------------------------------- #
    @property
    def last_activity(self) -> float:
        return self._last_activity

    def is_loaded(self) -> bool:
        return self._model is not None
