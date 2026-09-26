"""
MAVIS STT Engine — wraps faster-whisper for local speech-to-text.
Lazy-loads the model and supports idle unload to reclaim VRAM.
"""

from __future__ import annotations

import gc
import logging
import os
import re
import time

import numpy as np

logger = logging.getLogger("mavis.stt")

# Speech detection — which parts of the audio are someone talking.
#
# Whisper always produces fluent text, even from a fan: that's where
# "Thank you for watching, please subscribe..." came from. The earlier fix
# was a list of phrases to delete, and every new hallucination would have
# meant another entry, forever. Instead, Silero VAD — a small neural model
# trained to tell speech from everything else — marks the speech, and
# Whisper is given only that. No speech, no Whisper call, nothing to invent.
#
# Silero ships inside faster-whisper (with onnxruntime), so this adds no
# dependency. Tested 2026-09-22 against 40 s each of white, pink, brown and
# fan-like noise at three levels: zero seconds reported as speech in all
# twelve, while a spoken phrase mixed into the same noise was found with
# correct boundaries at every level where it was audible.
#
# min_silence 500 ms matches the Rust side's end-of-utterance pause, so a
# pause inside a sentence doesn't split it; speech_pad 300 ms keeps word
# onsets and endings Silero is conservative about.
SPEECH_GATE_OPTIONS = {
    "threshold": 0.5,
    "min_speech_duration_ms": 250,
    "min_silence_duration_ms": 500,
    "speech_pad_ms": 300,
}


def _speech_gate_enabled() -> bool:
    """MAVIS_SPEECH_GATE=0 turns the gate off — for diagnosing dropped speech only."""
    return os.environ.get("MAVIS_SPEECH_GATE", "1").lower() not in ("0", "false", "off")


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
        confidence_threshold: float = 0.45,
    ) -> None:
        self.model_size = model_size
        self.device = device
        self.compute_type = compute_type
        self.confidence_threshold = confidence_threshold
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

    def warm_up(self) -> None:
        """Pre-load the model so first request doesn't block on download."""
        print("[stt] Warming up model...", flush=True)
        self._load()
        print("[stt] Model warm-up complete.", flush=True)

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
    def transcribe(
        self,
        audio_bytes: bytes,
        sample_rate: int = 16000,
        bypass_confidence: bool = False,
    ) -> tuple[str, float]:
        """
        Transcribe raw PCM audio (float32, mono, 16 kHz).

        Args:
            audio_bytes: Raw float32 little-endian PCM.
            sample_rate: Expected sample rate (must match audio data).
            bypass_confidence: If True, skip the confidence gate (active listen).

        Returns:
            (text, confidence). Text is empty when there was no speech, or
            when what came back was too unreliable to use. Confidence is
            Whisper's own average, mapped to 0–1; the caller decides what
            to do with a borderline one.
        """
        self._load()
        self._last_activity = time.time()

        audio = np.frombuffer(audio_bytes, dtype=np.float32)
        if audio.size == 0:
            return "", 0.0

        duration = audio.size / sample_rate
        gate = _speech_gate_enabled()
        if gate:
            from faster_whisper.vad import VadOptions, get_speech_timestamps

            speech = get_speech_timestamps(audio, VadOptions(**SPEECH_GATE_OPTIONS))
            speech_s = sum(t["end"] - t["start"] for t in speech) / sample_rate
            logger.info(
                "STT input: %.2fs audio, %.2fs speech in %d region(s), peak=%.3f",
                duration,
                speech_s,
                len(speech),
                float(np.abs(audio).max()),
            )
            if not speech:
                print(
                    f"[stt] No speech in {duration:.2f}s of audio — not transcribing",
                    flush=True,
                )
                return "", 0.0
        else:
            logger.info("STT input: %.2fs audio, speech gate OFF", duration)

        segments, info = self._model.transcribe(
            audio,
            beam_size=5,
            best_of=5,
            # 2.0 → 1.0 (the standard setting). Patience widens the beam
            # search beyond beam_size; doubling it roughly doubled decode
            # time for no difference observed in the transcripts.
            patience=1.0,
            temperature=0.0,
            language="en",
            condition_on_previous_text=False,
            # Whisper sees only the speech regions (see SPEECH_GATE_OPTIONS).
            # Was False since 2026-08-15, disabled for "empty transcripts" in
            # the same session that fixed a dead microphone route and audio
            # truncated over the socket — so it was most likely being fed
            # no speech at all. Reversed 2026-09-22.
            vad_filter=gate,
            vad_parameters=SPEECH_GATE_OPTIONS if gate else None,
            # Bias the decoder toward the vocabulary actually used when
            # talking to a desktop assistant. This is not a filter: it tells
            # Whisper which rare words exist, so a weak signal resolves to
            # "clipboard" rather than "clayboard", and "MAVIS" rather than
            # "Ladies" or "Clayport".
            initial_prompt=(
                "MAVIS. clipboard, workspace, terminal, window, desktop "
                "environment, application, browser, Firefox, Brave, GNOME, "
                "niri, project, repository, volume, brightness."
            ),
        )

        # Consume generator so we can inspect per-segment confidence
        segments = list(segments)

        segment_confidences = []
        raw_parts = []
        for seg in segments:
            # Whisper's own verdict on a segment: likely no speech and not
            # confidently decoded, or suspiciously repetitive. The standard
            # thresholds from openai/whisper's transcribe(). This drops a
            # hallucinated segment on its own instead of relying on the
            # average over the whole utterance.
            no_speech = getattr(seg, "no_speech_prob", 0.0)
            logprob = getattr(seg, "avg_logprob", 0.0)
            compression = getattr(seg, "compression_ratio", 1.0)
            if (no_speech > 0.6 and logprob < -1.0) or compression > 2.4:
                logger.info(
                    "STT: dropping segment (no_speech=%.2f logprob=%.2f compression=%.2f): %s",
                    no_speech,
                    logprob,
                    compression,
                    seg.text[:80],
                )
                continue
            raw_parts.append(seg.text)
            # avg_logprob is (-inf, 0]. Map to [0, 1] via exp.
            conf = float(np.exp(seg.avg_logprob)) if hasattr(seg, "avg_logprob") else 1.0
            segment_confidences.append(conf)

        avg_confidence = float(np.mean(segment_confidences)) if segment_confidences else 0.0

        raw_text = " ".join(raw_parts).strip()
        text = self._deduplicate_repetition(raw_text)
        if not text:
            return "", 0.0

        # Confidence gate: drop ambient noise / hallucinations
        if not bypass_confidence and avg_confidence < self.confidence_threshold:
            logger.info(
                "STT: low confidence, dropping (avg=%.3f < %.3f): %s",
                avg_confidence,
                self.confidence_threshold,
                text[:80],
            )
            print(
                f"[stt] Low confidence ({avg_confidence:.3f} < {self.confidence_threshold}), "
                f"dropping utterance: '{text[:80]}'",
                flush=True,
            )
            return "", 0.0

        if text != raw_text:
            logger.info("STT dedup: '%s' -> '%s'", raw_text[:80], text[:80])

        logger.info(
            "Transcribed (%s, prob=%.2f, conf=%.2f): %s",
            info.language,
            info.language_probability,
            avg_confidence,
            text[:120],
        )
        return text, avg_confidence

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

        # Step through (sentence, punctuation) pairs. The range runs to
        # len(parts), not len(parts) - 1: re.split leaves any text after the
        # last punctuation mark as a final unpaired element, and stopping
        # early threw it away — an unpunctuated transcript came back empty,
        # and "Hi. open firefox" lost its command.
        for i in range(0, len(parts), 2):
            phrase = parts[i].strip()
            punct = parts[i + 1] if i + 1 < len(parts) else ""
            if not phrase:
                continue
            if phrase.lower() == prev_phrase:
                continue
            output.append((phrase + punct).strip())
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
