# mavis_worker/src/mavis/tts/kokoro.py
# Kokoro TTS engine — returns base64-encoded WAV bytes.
# Requires: kokoro>=0.9.4, soundfile>=0.12.1, numpy

import base64
import io
import time
import warnings

import numpy as np
import soundfile as sf


class KokoroEngine:
    def __init__(self):
        self.is_loaded = False
        self.last_activity = 0.0
        self._pipeline = None
        self._lang_code = "a"  # American English

    def _load(self) -> None:
        if self._pipeline is not None:
            return
        try:
            # Suppress kokoro's verbose spaCy download messages on first load
            with warnings.catch_warnings():
                warnings.simplefilter("ignore")
                from kokoro import KPipeline

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
