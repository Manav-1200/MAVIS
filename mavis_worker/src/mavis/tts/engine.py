class TTSEngine:
    """Stub TTS engine — TTS is now handled by the Rust runtime via Piper."""

    def __init__(self):
        self.is_loaded = False
        self.last_activity = 0.0

    def synthesize(self, text: str, voice=None, speed=1.0) -> str:
        # TTS is handled by Rust platform layer; this stub satisfies the interface.
        return ""

    def unload(self):
        self.is_loaded = False

    def warm_up(self):
        self.is_loaded = False
