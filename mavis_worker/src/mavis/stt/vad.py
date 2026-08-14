"""
Simple energy-threshold VAD. Zero extra dependencies.
Can run in Python (test / standalone mode) or be replaced by Rust VAD later.
"""

from __future__ import annotations

import logging
from collections import deque

import numpy as np

logger = logging.getLogger("mavis.vad")


class EnergyVAD:
    """
    Voice Activity Detection based on per-frame RMS energy.

    Frames below the threshold are considered silence.
    Speech is declared after ``min_speech_frames`` consecutive voiced frames.
    Speech ends after ``silence_frames`` consecutive silent frames.
    """

    def __init__(
        self,
        sample_rate: int = 16000,
        frame_duration_ms: int = 30,
        energy_threshold: float = 0.015,
        silence_duration_ms: float = 1200,
        min_speech_duration_ms: float = 300,
    ) -> None:
        self.sample_rate = sample_rate
        self.frame_size = int(sample_rate * frame_duration_ms / 1000)

        self.energy_threshold = energy_threshold
        self.silence_frames = max(1, int(silence_duration_ms / frame_duration_ms))
        self.min_speech_frames = max(1, int(min_speech_duration_ms / frame_duration_ms))

        # Rolling buffer of raw audio bytes
        self._buffer: deque[bytes] = deque()
        self._speech_frames = 0
        self._silence_frames = 0
        self._is_speaking = False

    # ------------------------------------------------------------------ #
    # Processing
    # ------------------------------------------------------------------ #
    def process(self, audio_chunk: bytes) -> bytes | None:
        """
        Feed a chunk of float32 PCM audio.

        Returns:
            Complete utterance bytes when silence is detected after speech,
            or None if still listening.
        """
        chunk = np.frombuffer(audio_chunk, dtype=np.float32)
        if chunk.size == 0:
            return None

        # Split chunk into frames
        frames = [chunk[i : i + self.frame_size] for i in range(0, len(chunk), self.frame_size)]

        for frame in frames:
            if len(frame) < self.frame_size:
                # Partial frame — buffer but do not process yet
                self._buffer.append(frame.tobytes())
                continue

            energy = float(np.sqrt(np.mean(frame**2)))
            self._buffer.append(frame.tobytes())

            if energy > self.energy_threshold:
                self._speech_frames += 1
                self._silence_frames = 0
                if self._speech_frames >= self.min_speech_frames:
                    self._is_speaking = True
            else:
                if self._is_speaking:
                    self._silence_frames += 1
                    if self._silence_frames >= self.silence_frames:
                        # Utterance complete
                        utterance = b"".join(self._buffer)
                        self.reset()
                        return utterance
                else:
                    # Trim pre-roll silence
                    while len(self._buffer) > self.silence_frames:
                        self._buffer.popleft()
                    self._speech_frames = max(0, self._speech_frames - 1)

        return None

    def reset(self) -> None:
        """Clear all state."""
        self._buffer.clear()
        self._speech_frames = 0
        self._silence_frames = 0
        self._is_speaking = False
