# mavis_worker/src/mavis/tts/engine.py
# TTS engine facade — delegates to Kokoro. Falls back to Piper if Kokoro fails.

import os


class TTSEngine:
    def __init__(self):
        self.is_loaded = False
        self.last_activity = 0.0
        self._kokoro = None
        self._piper = None

    def _init_kokoro(self):
        if self._kokoro is None:
            from mavis.tts.kokoro import KokoroEngine

            self._kokoro = KokoroEngine()
        return self._kokoro

    def synthesize(self, text: str, voice=None, speed=1.0) -> str:
        """
        Returns base64-encoded WAV audio.
        Tries Kokoro first; falls back to Piper if MAVIS_TTS_ENGINE=piper
        or if Kokoro is unavailable.
        """
        use_kokoro = os.environ.get("MAVIS_TTS_ENGINE", "kokoro").lower() != "piper"

        if use_kokoro:
            try:
                engine = self._init_kokoro()
                b64 = engine.synthesize(text, voice=voice, speed=speed)
                self.is_loaded = engine.is_loaded
                self.last_activity = engine.last_activity
                return b64
            except (RuntimeError, OSError, ImportError, ValueError) as e:
                print(f"[tts] Kokoro failed ({e}), falling back to Piper")

        return self._synthesize_piper(text)

    def _synthesize_piper(self, text: str) -> str:
        """Use piper-tts as a fallback; returns WAV base64."""
        import base64
        import subprocess
        import tempfile

        model = os.environ.get(
            "MAVIS_VOICE_MODEL",
            os.path.expanduser("~/.local/share/piper-voices/en_US-lessac-medium.onnx"),
        )
        config_path = model + ".json"
        if not os.path.exists(model) or not os.path.exists(config_path):
            raise RuntimeError(f"Piper model or config missing: {model}")

        with tempfile.NamedTemporaryFile(suffix=".wav", delete=False) as tmp:
            out_path = tmp.name

        try:
            subprocess.run(
                [
                    "piper",
                    "--model",
                    model,
                    "--output_file",
                    out_path,
                    "--config",
                    config_path,
                ],
                input=text.encode(),
                capture_output=True,
                check=True,
            )
            with open(out_path, "rb") as f:
                wav_bytes = f.read()
            return base64.b64encode(wav_bytes).decode("utf-8")
        finally:
            if os.path.exists(out_path):
                os.remove(out_path)

    def unload(self) -> None:
        if self._kokoro:
            self._kokoro.unload()
        self.is_loaded = False
        self.last_activity = 0.0

    def warm_up(self) -> None:
        try:
            self._init_kokoro().warm_up()
        except (RuntimeError, OSError, ImportError, ValueError) as e:
            print(f"[tts] Warm-up failed: {e}")
