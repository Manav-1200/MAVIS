"""
Microphone capture using sounddevice.
Primarily for testing / standalone Python STT mode.
Rust cpal is preferred for production to keep system I/O in the runtime.
"""

from __future__ import annotations

import logging
from collections.abc import Callable

import sounddevice as sd

logger = logging.getLogger("mavis.mic")


class MicCapture:
    """
    Non-blocking microphone capture with callback-based delivery.
    """

    def __init__(
        self,
        sample_rate: int = 16000,
        channels: int = 1,
        dtype: str = "float32",
        block_duration_ms: int = 30,
        device: int | None = None,
    ) -> None:
        self.sample_rate = sample_rate
        self.channels = channels
        self.dtype = dtype
        self.block_size = int(sample_rate * block_duration_ms / 1000)
        self.device = device

        self._stream: sd.RawInputStream | None = None
        self._callback: Callable[[bytes], None] | None = None
        self._running = False

    # ------------------------------------------------------------------ #
    # Control
    # ------------------------------------------------------------------ #
    def start(self, callback: Callable[[bytes], None]) -> None:
        """Begin capture. ``callback`` receives raw float32 bytes."""
        if self._running:
            return

        self._callback = callback
        self._running = True

        def _audio_callback(indata, frames, time_info, status):
            if status:
                logger.warning("Audio status: %s", status)
            if self._callback is not None:
                self._callback(indata.tobytes())

        self._stream = sd.RawInputStream(
            samplerate=self.sample_rate,
            blocksize=self.block_size,
            dtype=self.dtype,
            channels=self.channels,
            device=self.device,
            callback=_audio_callback,
        )
        self._stream.start()
        logger.info(
            "Mic capture started (device=%s, sr=%s)",
            self.device,
            self.sample_rate,
        )

    def stop(self) -> None:
        """Stop capture and release the stream."""
        self._running = False
        if self._stream is not None:
            self._stream.stop()
            self._stream.close()
            self._stream = None
        self._callback = None
        logger.info("Mic capture stopped.")

    # ------------------------------------------------------------------ #
    # Utilities
    # ------------------------------------------------------------------ #
    @staticmethod
    def list_devices() -> list[dict]:
        """Return a list of input devices."""
        devices = sd.query_devices()
        return [d for d in devices if d.get("max_input_channels", 0) > 0]

    @staticmethod
    def default_device() -> int | None:
        """Return the default input device index, or None."""
        try:
            return sd.default.device[0]  # type: ignore[index]
        except (AttributeError, IndexError):
            return None
