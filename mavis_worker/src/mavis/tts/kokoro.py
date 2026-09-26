# mavis_worker/src/mavis/tts/kokoro.py
# Kokoro TTS engine — returns base64-encoded WAV bytes.
# Requires: kokoro>=0.9.4, soundfile>=0.12.1, numpy

import base64
import io
import os
import time
import warnings
from pathlib import Path

import numpy as np
import soundfile as sf


class KokoroEngine:
    def __init__(self):
        self.is_loaded = False
        self.last_activity = 0.0
        self._pipeline = None
        self._lang_code = "a"  # American English

    @staticmethod
    def _prefer_local_weights() -> None:
        """
        Don't touch the network when the model is already downloaded.

        Kokoro asks Hugging Face about its weights on every load — visible
        as "unauthenticated requests to the HF Hub" in each run's log. For
        a local-first assistant that's a startup that fails when offline,
        for a model already sitting on disk.
        """
        if os.environ.get("HF_HUB_OFFLINE") is not None:
            return
        cache = (
            Path(os.environ.get("HF_HOME", Path.home() / ".cache" / "huggingface"))
            / "hub"
            / "models--hexgrad--Kokoro-82M"
        )
        if cache.exists():
            os.environ["HF_HUB_OFFLINE"] = "1"

    def _load(self) -> None:
        if self._pipeline is not None:
            return
        self._prefer_local_weights()
        try:
            # Loading kokoro prints spaCy download chatter, and building the
            # pipeline trips two harmless torch warnings (LSTM dropout with
            # one layer; weight_norm deprecation) inside kokoro's own model
            # code. None are actionable here, so they're kept out of the log
            # for the import and construction only.
            with warnings.catch_warnings():
                warnings.simplefilter("ignore")
                from kokoro import KPipeline

                try:
                    # Naming the repo explicitly silences "Defaulting
                    # repo_id to hexgrad/Kokoro-82M" on every load.
                    self._pipeline = KPipeline(
                        lang_code=self._lang_code, repo_id="hexgrad/Kokoro-82M"
                    )
                except TypeError:
                    # kokoro releases before repo_id existed.
                    self._pipeline = KPipeline(lang_code=self._lang_code)
            self.is_loaded = True
            print("[kokoro] Pipeline loaded.")
        except Exception as e:
            print(f"[kokoro] Failed to load pipeline: {e}")
            raise RuntimeError(f"Kokoro init failed: {e}") from e

    def synthesize(self, text: str, voice: str | None = None, speed: float = 1.0) -> str:
        """
        Synthesize text to speech.
        Returns: base64-encoded WAV bytes (RIFF header, 24 kHz, mono, PCM_16).
        """
        self._load()
        self.last_activity = time.time()

        voice = voice or "af_heart"
        # Clamp speed to Kokoro's supported range
        speed = max(0.5, min(speed, 2.0))

        try:
            generator = self._pipeline(text, voice=voice, speed=speed)
        except Exception as e:
            raise RuntimeError(f"Kokoro synthesis failed: {e}") from e

        segments: list[np.ndarray] = []
        for _gs, _ps, audio in generator:
            if audio is not None and len(audio) > 0:
                segments.append(audio)

        if not segments:
            raise RuntimeError("Kokoro produced no audio segments")

        full_audio = np.concatenate(segments)

        # Kokoro outputs float32 [-1, 1]. soundfile writes this as standard WAV.
        buffer = io.BytesIO()
        sf.write(buffer, full_audio, 24000, format="WAV", subtype="PCM_16")
        buffer.seek(0)
        wav_bytes = buffer.read()

        if len(wav_bytes) < 44:
            raise RuntimeError(f"Kokoro produced truncated WAV ({len(wav_bytes)} bytes)")

        # Validate RIFF header
        if wav_bytes[:4] != b"RIFF" or wav_bytes[8:12] != b"WAVE":
            raise RuntimeError("Kokoro audio missing valid WAV header")

        return base64.b64encode(wav_bytes).decode("utf-8")

    def unload(self) -> None:
        self._pipeline = None
        self.is_loaded = False
        print("[kokoro] Unloaded.")

    def warm_up(self) -> None:
        """Pre-load spaCy model and warm the torch cache."""
        self._load()
        # Synthesize one silent token to force lazy init
        self.synthesize("hello", voice="af_heart")
        print("[kokoro] Warm-up complete.")
