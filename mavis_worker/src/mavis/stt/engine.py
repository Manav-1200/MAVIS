"""
MAVIS STT Engine — wraps faster-whisper for local speech-to-text.
Lazy-loads the model and supports idle unload to reclaim VRAM.
"""

from __future__ import annotations

import gc
import logging
import re
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

    # ------------------------------------------------------------------ #
    # Lifecycle
    # ------------------------------------------------------------------ #
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

    # ------------------------------------------------------------------ #
    # Inference
    # ------------------------------------------------------------------ #
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

        # Trim trailing silence to prevent echo/repetition artifacts
        audio = self._trim_trailing_silence(audio, sample_rate=sample_rate)

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
            best_of=5,
            patience=2.0,
            temperature=0.0,
            language="en",
            condition_on_previous_text=False,
            vad_filter=False,  # Rust VAD already segmented; don't double-filter
        )

        raw_text = " ".join(seg.text for seg in segments).strip()
        text = self._deduplicate_repetition(raw_text)

        if text != raw_text:
            logger.info("STT dedup: '%s' -> '%s'", raw_text[:80], text[:80])

        logger.info(
            "Transcribed (%s, prob=%.2f): %s",
            info.language,
            info.language_probability,
            text[:120],
        )
        return text

    @staticmethod
    def _trim_trailing_silence(
        audio: np.ndarray,
        sample_rate: int = 16000,
        threshold: float = 0.01,
        padding_ms: int = 50,
    ) -> np.ndarray:
        """Trim trailing silence from normalized float32 audio."""
        if audio.size == 0:
            return audio

        above_threshold = np.abs(audio) > threshold
        if not np.any(above_threshold):
            return audio

        last_speech_idx = int(np.where(above_threshold)[0][-1])
        padding_samples = int(sample_rate * padding_ms / 1000)
        end_idx = min(last_speech_idx + padding_samples, audio.size)
        trimmed = audio[:end_idx]

        logger.debug(
            "Trimmed trailing silence: %d -> %d samples (%.2f s -> %.2f s)",
            audio.size,
            trimmed.size,
            audio.size / sample_rate,
            trimmed.size / sample_rate,
        )
        return trimmed

    @staticmethod
    def _deduplicate_repetition(text: str) -> str:
        """
        Collapse repeated phrases that faster-whisper hallucinates.
        Handles both sentence-level ('Hi. Hi.') and phrase-level loops
        ('go to the airport... go to the airport...').
        """
        if not text or len(text.split()) < 4:
            return text

        words = text.split()
        cleaned = words[:]
        changed = False

        # --- Pass 1: phrase-level dedup (4+ word windows, 3+ repeats) ---
        for window in range(4, len(words) // 3 + 1):
            i = 0
            pass_cleaned = []
            while i < len(cleaned):
                phrase = cleaned[i : i + window]
                if len(phrase) < window:
                    pass_cleaned.extend(phrase)
                    break

                # Count consecutive repeats
                repeat_count = 1
                j = i + window
                while j + window <= len(cleaned) and cleaned[j : j + window] == phrase:
                    repeat_count += 1
                    j += window

                if repeat_count >= 3:
                    # Strong hallucination — keep one
                    pass_cleaned.extend(phrase)
                    i = j
                    changed = True
                elif repeat_count == 2:
                    # Possible emphasis — keep two
                    pass_cleaned.extend(phrase * 2)
                    i = j
                    changed = True
                else:
                    pass_cleaned.append(cleaned[i])
                    i += 1

            if changed:
                cleaned = pass_cleaned
                break  # One pass is enough; avoids over-aggression

        result = " ".join(cleaned)

        # --- Pass 2: sentence-level dedup (safety net) ---
        parts = re.split(r"([.!?]+(?:\s+|$))", result)
        output = []
        prev_phrase = None

        for i in range(0, len(parts) - 1, 2):
            phrase = parts[i].strip()
            punct = parts[i + 1] if i + 1 < len(parts) else ""
            if not phrase:
                continue
            if phrase.lower() == prev_phrase:
                continue
            output.append(phrase + punct)
            prev_phrase = phrase.lower()

        return " ".join(output)

    # ------------------------------------------------------------------ #
    # Idle monitoring
    # ------------------------------------------------------------------ #
    @property
    def last_activity(self) -> float:
        return self._last_activity

    def is_loaded(self) -> bool:
        return self._model is not None
