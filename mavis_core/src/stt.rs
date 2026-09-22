// mavis_core/src/stt.rs
//! MAVIS STT Manager — Rust runtime side

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, SampleFormat, SampleRate, SupportedStreamConfig};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use log::{error, info, warn};

use crate::event_bus::EventBus;
use crate::models::event::{Event, EventType};

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct SttConfig {
    pub sample_rate: u32,
    pub silence_duration_ms: u64,
    pub min_speech_duration_ms: u64,
    pub frame_duration_ms: u64,
    pub soft_max_duration_ms: u64,
    pub grace_silence_ms: u64,
    pub hard_max_duration_ms: u64,
    pub min_max_energy: f32,
}

impl Default for SttConfig {
    fn default() -> Self {
        Self {
            sample_rate: 16000,
            // 600ms → 500ms. This is dead time on every exchange: the user
            // has stopped talking and MAVIS is still waiting to be sure.
            // Floor is set by measurement, not preference — the longest
            // sub-threshold dip observed *inside* real speech was 450ms at
            // the current 0.06 end threshold, so anything at or below that
            // splits sentences mid-thought. 500ms keeps a 50ms margin.
            // Going lower means raising SPEECH_END_THRESHOLD first.
            silence_duration_ms: 500,
            min_speech_duration_ms: 250,
            frame_duration_ms: 30,
            // Past this much actual speech, switch to a short grace-period
            // pause detector instead of waiting for a full 600ms pause —
            // ends at the next natural gap instead of chopping mid-word.
            soft_max_duration_ms: 20000,
            grace_silence_ms: 150,
            // Absolute ceiling regardless of pauses. Measured in buffered
            // wall-clock time (not speech-frame count) since that's what
            // actually bounds memory — a rare last resort, not the normal path.
            // 45 s → 30 s: nobody gives a desktop assistant a 30-second
            // command, and when this fires it's always been noise.
            hard_max_duration_ms: 30000,
            // Measured speech median is 0.104, silence median 0.033. This
            // gate rejects whole utterances that never got loud enough to
            // be real speech.
            min_max_energy: 0.10,
        }
    }
}

// Thresholds — absolute minimums plus a noise-relative part.
//
// The minimums are the old fixed thresholds, measured in a quiet room on
// this machine (silence median 0.033, peak 0.058; speech onset ~0.15,
// quiet syllables dipping to 0.04). They still decide everything there,
// because the relative part only takes over once the room is louder.
//
// Fixed thresholds alone failed the moment the room wasn't quiet: with the
// mic at 0.6 and fans spinning up while the model loaded, ambient sat
// above the 0.06 end threshold, so MAVIS never heard the user stop — it
// recorded to the 45 s hard ceiling and Whisper invented a YouTube outro
// from the noise. The old noise floor couldn't follow: it was capped at
// 0.045 and reset to 0.035 after every utterance.
//
// Ratios: in the quiet room 1.8x and 1.5x of the 0.033 floor land at 0.059
// and 0.050 — under the minimums, so behaviour there is unchanged. Higher
// ratios were tried in simulation and went deaf to speech only ~2.4x louder
// than the room; these still catch it, and steady noise that wobbles ±90%
// frame to frame produced no false utterances over a minute.
const SPEECH_START_MIN: f32 = 0.075;
const SPEECH_END_MIN: f32 = 0.06;
const START_RATIO: f32 = 1.8;
const END_RATIO: f32 = 1.5;

/// Where the floor starts before calibration: the measured silence median.
const INITIAL_NOISE_FLOOR: f32 = 0.035;
/// Upper bound on the floor. At 0.25 the start threshold is already 0.45;
/// beyond that the microphone is clipping and no threshold helps.
const MAX_NOISE_FLOOR: f32 = 0.25;

/// Frames (30 ms) spent measuring the room after the stream settles,
/// before any detection. Starting from a guessed floor is what let the
/// very first frames of a noisy room read as speech.
const CALIBRATION_FRAMES: usize = 30;

/// Floor tracking between utterances: follows quiet quickly and noise
/// slowly, so a word's onset can't drag the floor up under itself, while a
/// fan that spins up is still learned in a second or two.
const FLOOR_FALL_RATE: f32 = 0.2;
const FLOOR_RISE_RATE: f32 = 0.02;

/// Energy smoothing (EMA weight of the newest frame). Raw 30 ms frames of
/// steady noise flicker above and below any threshold, and one loud frame
/// used to reset the silence count; smoothing turns fan noise into a flat
/// line and fills the brief dips inside words. ~70 ms time constant, so an
/// utterance end is noticed at most a frame or two later.
const SMOOTHING: f32 = 0.4;

/// While speaking: if the quietest moment in this many frames is still
/// above the end threshold, it isn't speech that's holding the utterance
/// open — real speech always dips between words and breaths. It's the room.
/// Re-measure the floor from that quietest moment and let the utterance end.
/// ~2.7 s: this is what turns a 45 s stuck recording into ~3 s.
const REBASELINE_FRAMES: usize = 90;

/// Audio kept after the last loud frame when trimming an utterance's tail.
const TAIL_PADDING_MS: usize = 300;

/// Ignore audio for this long after the input stream opens. The device
/// emits a full-scale click on startup (measured max_energy=1.000), which
/// the VAD would otherwise ship as a 1.3s "utterance" for Whisper to
/// hallucinate words from.
const STREAM_SETTLE_TIME: Duration = Duration::from_millis(1500);

/// Shortest utterance worth transcribing. A real spoken phrase runs well
/// over a second; anything briefer is a click, a keypress or a door — loud
/// enough to pass the energy gates, but not speech. Whisper responds to
/// such fragments by inventing fluent text, so they're dropped here.
/// Includes the up-to-1 s of audio kept from before speech started.
const MIN_UTTERANCE_SAMPLES: usize = 24000; // 1.5s at 16kHz

// ---------------------------------------------------------------------------
// VAD — smoothed energy with a noise floor that follows the room
// ---------------------------------------------------------------------------

struct EnergyVad {
    frame_size: usize,
    silence_threshold_frames: usize,
    min_speech_threshold_frames: usize,
    soft_max_frames: usize,
    grace_silence_frames: usize,
    hard_max_samples: usize,
    tail_padding_samples: usize,
    past_soft_ceiling: bool,
    noise_floor: f32,
    /// Frames still to measure before detection starts.
    calibration_left: usize,
    calibration: Vec<f32>,
    smoothed: f32,
    /// Samples from the last callback that didn't fill a whole frame.
    /// Previously appended to the buffer unanalysed, which also shifted
    /// every later frame boundary.
    pending: Vec<f32>,
    buffer: VecDeque<f32>,
    /// Per frame since speech started: (buffer length after it, smoothed
    /// energy). Used to trim the tail once the utterance ends.
    trace: Vec<(usize, f32)>,
    /// Smoothed energies of the most recent frames while speaking.
    recent: VecDeque<f32>,
    speech_frames: usize,
    silence_frames: usize,
    pub is_speaking: bool,
    max_energy_seen: f32,
    pub last_max_energy: f32,
    sample_rate: usize,
    /// MAVIS_VAD_DEBUG=1 reports what the microphone is actually producing.
    /// Off by default (it logs every ~3 s), but the one diagnostic that
    /// distinguishes "MAVIS is broken" from "the mic is muted" — which
    /// cost a whole debugging session when it wasn't available.
    debug_energy: bool,
    debug_frames: u64,
    debug_window_max: f32,
}

impl EnergyVad {
    fn new(cfg: &SttConfig) -> Self {
        let frame_size = (cfg.sample_rate as usize * cfg.frame_duration_ms as usize) / 1000;
        Self {
            frame_size,
            silence_threshold_frames: ((cfg.silence_duration_ms / cfg.frame_duration_ms).max(1))
                as usize,
            min_speech_threshold_frames: ((cfg.min_speech_duration_ms / cfg.frame_duration_ms)
                .max(1)) as usize,
            soft_max_frames: ((cfg.soft_max_duration_ms / cfg.frame_duration_ms).max(1))
                as usize,
            grace_silence_frames: ((cfg.grace_silence_ms / cfg.frame_duration_ms).max(1))
                as usize,
            hard_max_samples: (cfg.sample_rate as usize) * (cfg.hard_max_duration_ms as usize)
                / 1000,
            tail_padding_samples: cfg.sample_rate as usize * TAIL_PADDING_MS / 1000,
            past_soft_ceiling: false,
            noise_floor: INITIAL_NOISE_FLOOR,
            calibration_left: CALIBRATION_FRAMES,
            calibration: Vec::with_capacity(CALIBRATION_FRAMES),
            smoothed: 0.0,
            pending: Vec::new(),
            buffer: VecDeque::new(),
            trace: Vec::new(),
            recent: VecDeque::with_capacity(REBASELINE_FRAMES + 1),
            speech_frames: 0,
            silence_frames: 0,
            is_speaking: false,
            max_energy_seen: 0.0,
            last_max_energy: 0.0,
            sample_rate: cfg.sample_rate as usize,
            debug_energy: matches!(
                std::env::var("MAVIS_VAD_DEBUG").as_deref(),
                Ok("1") | Ok("true")
            ),
            debug_frames: 0,
            debug_window_max: 0.0,
        }
    }

    fn start_threshold(&self) -> f32 {
        SPEECH_START_MIN.max(self.noise_floor * START_RATIO)
    }

    fn end_threshold(&self) -> f32 {
        SPEECH_END_MIN.max(self.noise_floor * END_RATIO)
    }

    fn process(&mut self, samples: &[f32]) -> Option<Vec<f32>> {
        self.pending.extend_from_slice(samples);
        let mut offset = 0;
        let mut result = None;
        while self.pending.len() - offset >= self.frame_size {
            let end = offset + self.frame_size;
            // Copy out so process_frame can borrow self mutably. 480 floats.
            let frame: Vec<f32> = self.pending[offset..end].to_vec();
            offset = end;
            if let Some(u) = self.process_frame(&frame) {
                result = Some(u);
                // Whatever follows belongs to the next utterance; keep it
                // pending rather than processing it against a fresh state
                // in this same call — simpler, and at most one callback late.
                break;
            }
        }
        self.pending.drain(..offset);
        result
    }

    fn process_frame(&mut self, frame: &[f32]) -> Option<Vec<f32>> {
        let energy = (frame.iter().map(|s| s * s).sum::<f32>() / frame.len() as f32).sqrt();
        self.smoothed = if self.smoothed == 0.0 {
            energy
        } else {
            self.smoothed + SMOOTHING * (energy - self.smoothed)
        };
        let level = self.smoothed;

        // Measure the room before listening for speech.
        if self.calibration_left > 0 {
            self.calibration.push(level);
            self.calibration_left -= 1;
            if self.calibration_left == 0 {
                let mut sorted = self.calibration.clone();
                sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                let median = sorted[sorted.len() / 2];
                self.noise_floor = median.clamp(0.001, MAX_NOISE_FLOOR);
                self.calibration = Vec::new();
                info!(
                    "VAD: calibrated — noise_floor={:.4}, start={:.4}, end={:.4}",
                    self.noise_floor,
                    self.start_threshold(),
                    self.end_threshold()
                );
            }
            return None;
        }

        self.max_energy_seen = self.max_energy_seen.max(energy);

        if self.debug_energy {
            self.debug_frames += 1;
            self.debug_window_max = self.debug_window_max.max(energy);
            // `%`, not is_multiple_of(): that needs Rust 1.87, MSRV is 1.85.
            #[allow(clippy::manual_is_multiple_of)]
            let report = self.debug_frames % 100 == 0;
            if report {
                info!(
                    "VAD-DEBUG: peak={:.4} level={:.4} start={:.4} end={:.4} \
                     noise_floor={:.4} speaking={}",
                    self.debug_window_max,
                    level,
                    self.start_threshold(),
                    self.end_threshold(),
                    self.noise_floor,
                    self.is_speaking
                );
                self.debug_window_max = 0.0;
            }
        }

        self.buffer.extend(frame);

        // Hysteresis: once speaking, a frame only counts as silence if it
        // drops below the LOWER end threshold. Before speaking, it takes
        // the HIGHER start threshold to begin.
        let threshold = if self.is_speaking {
            self.end_threshold()
        } else {
            self.start_threshold()
        };

        if self.is_speaking {
            self.trace.push((self.buffer.len(), level));
            self.recent.push_back(level);
            if self.recent.len() > REBASELINE_FRAMES {
                self.recent.pop_front();
            }
            if self.recent.len() == REBASELINE_FRAMES {
                let quietest = self.recent.iter().copied().fold(f32::INFINITY, f32::min);
                if quietest > self.end_threshold() {
                    self.noise_floor = quietest.min(MAX_NOISE_FLOOR);
                    self.recent.clear();
                    info!(
                        "VAD: room got louder mid-utterance — noise_floor now {:.4} (end={:.4})",
                        self.noise_floor,
                        self.end_threshold()
                    );
                }
            }
        }

        if level > threshold {
            self.speech_frames += 1;
            self.silence_frames = 0;
            if self.speech_frames >= self.min_speech_threshold_frames && !self.is_speaking {
                info!(
                    "VAD: SPEECH START (level={:.4}, threshold={:.4}, noise_floor={:.4})",
                    level, threshold, self.noise_floor
                );
                self.is_speaking = true;
            }
            if self.speech_frames >= self.soft_max_frames && !self.past_soft_ceiling {
                self.past_soft_ceiling = true;
                info!("VAD: past soft ceiling, switching to grace-period pause detection");
            }
        } else if self.is_speaking {
            self.silence_frames += 1;
            // Once past the soft ceiling, end at the next brief gap
            // instead of waiting for a full natural pause.
            let required_silence = if self.past_soft_ceiling {
                self.grace_silence_frames
            } else {
                self.silence_threshold_frames
            };
            if self.silence_frames >= required_silence {
                let reason = if self.past_soft_ceiling { "SPEECH END (grace)" } else { "SPEECH END" };
                return self.finish(reason);
            }
        } else {
            let rate = if level < self.noise_floor { FLOOR_FALL_RATE } else { FLOOR_RISE_RATE };
            self.noise_floor = (self.noise_floor + rate * (level - self.noise_floor))
                .clamp(0.001, MAX_NOISE_FLOOR);
            while self.buffer.len() > self.sample_rate {
                self.buffer.pop_front();
            }
            self.speech_frames = self.speech_frames.saturating_sub(1);
        }

        // Absolute last resort — fires regardless of pauses.
        if self.is_speaking && self.buffer.len() >= self.hard_max_samples {
            return self.finish("FORCED END — hard ceiling");
        }
        None
    }

    /// End the utterance: trim trailing room noise, then hand it over —
    /// or drop it if nothing in it was ever loud enough to be speech
    /// against the room as it's now understood.
    fn finish(&mut self, reason: &str) -> Option<Vec<f32>> {
        let end = self.end_threshold();
        let start = self.start_threshold();
        let last_loud = self.trace.iter().rev().find(|(_, e)| *e > end).map(|(pos, _)| *pos);
        let loudest = self.trace.iter().map(|(_, e)| *e).fold(0.0f32, f32::max);

        let mut utterance: Vec<f32> = self.buffer.drain(..).collect();
        let before = utterance.len();
        if let Some(pos) = last_loud {
            let cut = (pos + self.tail_padding_samples).min(utterance.len());
            utterance.truncate(cut);
        }
        self.last_max_energy = self.max_energy_seen;
        info!(
            "VAD: {} ({} samples, trimmed from {}, max_energy={:.3}, noise_floor={:.4})",
            reason,
            utterance.len(),
            before,
            self.max_energy_seen,
            self.noise_floor
        );
        let is_noise = loudest <= start;
        self.reset();
        if is_noise {
            info!(
                "VAD: dropping — never rose above the start threshold ({:.4} ≤ {:.4}); it was the room",
                loudest, start
            );
            return None;
        }
        Some(utterance)
    }

    /// Forget the current utterance. The noise floor is deliberately kept:
    /// resetting it to a constant after every utterance is what made MAVIS
    /// "hear" speech the instant it finished talking in a noisy room.
    fn reset(&mut self) {
        // `pending` is left alone: it's under one frame of audio that
        // belongs to whatever comes next, and process() is mid-way through
        // it when an utterance ends.
        self.buffer.clear();
        self.trace.clear();
        self.recent.clear();
        self.speech_frames = 0;
        self.silence_frames = 0;
        self.is_speaking = false;
        self.max_energy_seen = 0.0;
        self.past_soft_ceiling = false;
    }
}

// ---------------------------------------------------------------------------
// Handle
// ---------------------------------------------------------------------------

pub struct SttHandle {
    running: Arc<AtomicBool>,
    _stream: cpal::Stream,
}

impl SttHandle {
    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }
}

// ---------------------------------------------------------------------------
// Device selection
// ---------------------------------------------------------------------------

fn select_input_device(host: &cpal::Host) -> Option<Device> {
    // If MAVIS_AUDIO_DEVICE is set, try exact name match first
    if let Ok(env_device) = std::env::var("MAVIS_AUDIO_DEVICE") {
        if let Ok(devs) = host.input_devices() {
            for d in devs {
                if let Ok(name) = d.name() {
                    if name == env_device {
                        info!("STT: using MAVIS_AUDIO_DEVICE exact match: {}", name);
                        return Some(d);
                    }
                }
            }
        }
        warn!(
            "MAVIS_AUDIO_DEVICE='{}' not found among input devices, falling back to scoring",
            env_device
        );
    }

    let devices: Vec<(Device, String)> = match host.input_devices() {
        Ok(devs) => devs.filter_map(|d| d.name().ok().map(|n| (d, n))).collect(),
        Err(e) => {
            warn!("Failed to enumerate input devices: {}", e);
            return host.default_input_device();
        }
    };

    info!("=== CPAL Input Devices ===");
    for (idx, (_, name)) in devices.iter().enumerate() {
        info!("  [{}] {}", idx, name);
    }
    info!("==========================");

    let score = |name: &str| {
        let lower = name.to_lowercase();
        if lower.contains("front") && lower.contains("generic") { return 100; }
        if lower.contains("sysdefault") && lower.contains("generic") { return 90; }
        if lower.contains("analog") && !lower.contains("hdmi") { return 80; }
        if !lower.contains("bluez") && !lower.contains("hdmi") && !lower.contains("monitor") { return 70; }
        if lower == "default" { return 60; }
        0
    };

    let mut best: Option<(Device, String)> = None;
    let mut best_score = -1;
    for (device, name) in devices {
        let s = score(&name);
        if s > best_score {
            best_score = s;
            best = Some((device, name));
        }
    }

    if let Some((_, ref name)) = best {
        info!("STT selected device: {} (score={})", name, best_score);
    }

    best.map(|(d, _)| d).or_else(|| host.default_input_device())
}

// ---------------------------------------------------------------------------
// Manager
// ---------------------------------------------------------------------------

pub struct SttManager {
    config: SttConfig,
}

impl SttManager {
    pub fn new(config: SttConfig) -> Self {
        Self { config }
    }

    /// Start listening.
    ///
    /// Returns Err rather than panicking when audio is unavailable: a
    /// missing or busy microphone should cost MAVIS its ears, not its life.
    /// Previously any of these failures aborted the whole process, taking
    /// memory, context and the orb down with them.
    pub fn start(
        self,
        bus: Arc<EventBus>,
        speech_start_tx: Option<mpsc::Sender<()>>,
        tts_active: Arc<AtomicBool>,
    ) -> anyhow::Result<(SttHandle, mpsc::Receiver<Vec<f32>>, mpsc::Receiver<f32>)> {
        let running = Arc::new(AtomicBool::new(true));
        let running_stream = running.clone();
        let tts_active_stream = tts_active;

        let (tx, rx) = mpsc::channel::<Vec<f32>>(4);
        let (energy_tx, energy_rx) = mpsc::channel::<f32>(64);
        let vad = Arc::new(Mutex::new(EnergyVad::new(&self.config)));
        let config = self.config.clone();

        let host = cpal::default_host();

        let device = select_input_device(&host)
            .ok_or_else(|| anyhow::anyhow!("no audio input device available"))?;

        let configs: Vec<cpal::SupportedStreamConfigRange> = device
            .supported_input_configs()
            .map_err(|e| anyhow::anyhow!("could not query input configs: {}", e))?
            .collect();

        // Prefer f32, then the integer formats the callback converts. Prefer
        // 16 kHz (no resampling), then common rates that divide or nearly
        // divide cleanly, and only then the device maximum — previously a
        // device without 16 kHz f32 was opened at its maximum rate, which
        // can be 192 kHz: twelve times the audio to resample for nothing.
        let format_rank = |f: SampleFormat| match f {
            SampleFormat::F32 => 0,
            SampleFormat::I16 => 1,
            SampleFormat::I32 => 2,
            SampleFormat::U16 => 3,
            _ => 9,
        };
        let pick_rate = |c: &cpal::SupportedStreamConfigRange| {
            for r in [16000u32, 48000, 32000, 44100] {
                if c.min_sample_rate().0 <= r && r <= c.max_sample_rate().0 {
                    return SampleRate(r);
                }
            }
            c.max_sample_rate()
        };
        let stream_config: SupportedStreamConfig = configs
            .iter()
            .filter(|c| format_rank(c.sample_format()) < 9)
            .min_by_key(|c| {
                let rate = pick_rate(c).0;
                (format_rank(c.sample_format()), rate != 16000, rate, c.channels())
            })
            .map(|c| (*c).with_sample_rate(pick_rate(c)))
            .ok_or_else(|| anyhow::anyhow!("no supported input configuration"))?;

        info!("STT stream config: {:?}", stream_config);

        let sample_rate = stream_config.sample_rate().0;
        let channels = stream_config.channels() as usize;
        let sample_format = stream_config.sample_format();

        let err_fn = |err| error!("STT stream error: {}", err);

        let mut capture = Capture {
            running: running_stream,
            tts_active: tts_active_stream,
            stream_opened_at: Instant::now(),
            was_tts_active: false,
            post_tts_cooldown: None,
            channels,
            device_rate: sample_rate,
            target_rate: config.sample_rate,
            min_max_energy: config.min_max_energy,
            vad,
            tx,
            energy_tx,
            bus,
            speech_start: speech_start_tx,
        };

        // Most devices offer f32, but some USB microphones and plain ALSA
        // devices only offer integer formats. Those used to fail with
        // "unsupported sample format" and leave MAVIS deaf; now they're
        // converted to f32 at the edge and share one pipeline.
        let cfg: cpal::StreamConfig = stream_config.into();
        let stream = match sample_format {
            SampleFormat::F32 => device.build_input_stream(
                &cfg,
                move |data: &[f32], _: &cpal::InputCallbackInfo| capture.on_samples(data),
                err_fn,
                None,
            ),
            SampleFormat::I16 => device.build_input_stream(
                &cfg,
                move |data: &[i16], _: &cpal::InputCallbackInfo| {
                    let f: Vec<f32> = data.iter().map(|&s| s as f32 / 32768.0).collect();
                    capture.on_samples(&f)
                },
                err_fn,
                None,
            ),
            SampleFormat::U16 => device.build_input_stream(
                &cfg,
                move |data: &[u16], _: &cpal::InputCallbackInfo| {
                    let f: Vec<f32> =
                        data.iter().map(|&s| (s as f32 - 32768.0) / 32768.0).collect();
                    capture.on_samples(&f)
                },
                err_fn,
                None,
            ),
            SampleFormat::I32 => device.build_input_stream(
                &cfg,
                move |data: &[i32], _: &cpal::InputCallbackInfo| {
                    let f: Vec<f32> =
                        data.iter().map(|&s| s as f32 / 2_147_483_648.0).collect();
                    capture.on_samples(&f)
                },
                err_fn,
                None,
            ),
            other => {
                return Err(anyhow::anyhow!("unsupported sample format: {:?}", other));
            }
        }
        .map_err(|e| anyhow::anyhow!("could not build input stream: {}", e))?;

        stream
            .play()
            .map_err(|e| anyhow::anyhow!("could not start audio stream: {}", e))?;
        info!("STT listening active — measuring the room for the first second.");

        let handle = SttHandle { running, _stream: stream };
        Ok((handle, rx, energy_rx))
    }
}

// ---------------------------------------------------------------------------
// Capture — everything the audio callback does, independent of sample type
// ---------------------------------------------------------------------------

struct Capture {
    running: Arc<AtomicBool>,
    tts_active: Arc<AtomicBool>,
    /// Opening the audio stream produces a full-scale click/pop on some
    /// hardware (observed max_energy=1.000 one second after startup).
    /// Audio is ignored until the device settles.
    stream_opened_at: Instant,
    was_tts_active: bool,
    post_tts_cooldown: Option<Instant>,
    channels: usize,
    device_rate: u32,
    target_rate: u32,
    min_max_energy: f32,
    vad: Arc<Mutex<EnergyVad>>,
    tx: mpsc::Sender<Vec<f32>>,
    energy_tx: mpsc::Sender<f32>,
    bus: Arc<EventBus>,
    speech_start: Option<mpsc::Sender<()>>,
}

impl Capture {
    fn publish_state(&self, state: &str) {
        self.bus.publish(Event {
            id: uuid::Uuid::new_v4(),
            timestamp: chrono::Utc::now(),
            source: "stt".to_string(),
            event_type: EventType::UiStateChange,
            payload: serde_json::json!({ "state": state }),
        });
    }

    /// A poisoned lock must not kill the audio thread — recover the guard.
    ///
    /// This MUST be a `match`, not `if let Ok(..) {} else if let Err(..) {}`.
    /// On edition 2021 the scrutinee temporary of an `if let` lives until
    /// the end of the whole if/else chain, so in the Err arm the first
    /// `lock()`'s PoisonError — which owns the guard — was still alive when
    /// the second `lock()` ran. std::sync::Mutex is not reentrant: that
    /// deadlocked the audio callback thread outright.
    fn reset_vad(&self) {
        match self.vad.lock() {
            Ok(mut g) => g.reset(),
            Err(p) => p.into_inner().reset(),
        }
    }

    fn on_samples(&mut self, data: &[f32]) {
        if !self.running.load(Ordering::Relaxed) {
            return;
        }
        if self.stream_opened_at.elapsed() < STREAM_SETTLE_TIME {
            return;
        }

        // TTS is playing — discard everything to prevent echo contamination.
        if self.tts_active.load(Ordering::Relaxed) {
            self.was_tts_active = true;
            self.reset_vad();
            return;
        }

        // TTS just ended — reset and let room echo decay before listening.
        if self.was_tts_active {
            self.reset_vad();
            self.was_tts_active = false;
            self.post_tts_cooldown = Some(Instant::now());
        }
        if let Some(start) = self.post_tts_cooldown {
            if start.elapsed() < Duration::from_millis(250) {
                return;
            }
            self.post_tts_cooldown = None;
        }

        let mono: Vec<f32> = if self.channels <= 1 {
            data.to_vec()
        } else {
            data.chunks(self.channels)
                .map(|c| c.iter().sum::<f32>() / self.channels as f32)
                .collect()
        };
        let resampled = if self.device_rate == self.target_rate {
            mono
        } else {
            resample_linear(&mono, self.device_rate, self.target_rate)
        };

        if !resampled.is_empty() {
            let frame_energy = (resampled.iter().map(|s| s * s).sum::<f32>()
                / resampled.len() as f32)
                .sqrt();
            let _ = self.energy_tx.try_send(frame_energy);
        }

        let (was_speaking, result, max_energy, now_speaking) = {
            let mut g = match self.vad.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            let was = g.is_speaking;
            let r = g.process(&resampled);
            (was, r, g.last_max_energy, g.is_speaking)
        };

        if let Some(utterance) = result {
            if utterance.len() < MIN_UTTERANCE_SAMPLES {
                info!(
                    "STT: dropping short utterance ({} samples < {})",
                    utterance.len(),
                    MIN_UTTERANCE_SAMPLES
                );
                self.publish_state("idle");
                return;
            }
            if max_energy < self.min_max_energy {
                info!(
                    "STT: dropping noise utterance (max_energy={:.3} < {:.3})",
                    max_energy, self.min_max_energy
                );
                self.publish_state("idle");
                return;
            }
            info!("STT: shipping utterance ({} samples)", utterance.len());
            let _ = self.tx.try_send(utterance);
            self.publish_state("thinking");
        } else if was_speaking && !now_speaking {
            // The VAD ended an utterance and dropped it as room noise.
            self.publish_state("idle");
        } else if !was_speaking && now_speaking {
            if let Some(ref sstx) = self.speech_start {
                let _ = sstx.try_send(());
            }
            self.publish_state("listening");
        }
    }
}

fn resample_linear(input: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if from_rate == to_rate {
        return input.to_vec();
    }
    let ratio = from_rate as f32 / to_rate as f32;
    let out_len = (input.len() as f32 / ratio) as usize;
    (0..out_len)
        .map(|i| {
            let src_idx = i as f32 * ratio;
            let src_floor = src_idx.floor() as usize;
            let frac = src_idx - src_idx.floor();
            let a = input.get(src_floor).copied().unwrap_or(0.0);
            let b = input.get(src_floor + 1).copied().unwrap_or(a);
            a + frac * (b - a)
        })
        .collect()
}

#[cfg(test)]
mod vad_tests {
    use super::*;

    const FRAME: usize = 480; // 30 ms at 16 kHz

    /// Deterministic noise with RMS close to `rms` (uniform ±rms*√3).
    fn noise(rms: f32, frames: usize, seed: &mut u32) -> Vec<f32> {
        let amp = rms * 3f32.sqrt();
        (0..frames * FRAME)
            .map(|_| {
                *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let u = (*seed >> 8) as f32 / (1u32 << 24) as f32; // [0,1)
                (u * 2.0 - 1.0) * amp
            })
            .collect()
    }

    /// Speech-like: syllables at `loud` separated by brief dips to the
    /// room level, over `ms` milliseconds.
    fn speech(loud: f32, room: f32, ms: usize, seed: &mut u32) -> Vec<f32> {
        let mut out = Vec::new();
        let frames = ms / 30;
        let mut i = 0;
        while i < frames {
            out.extend(noise(loud, 6.min(frames - i), seed)); // 180 ms syllable
            i += 6;
            if i < frames {
                out.extend(noise(room, 2.min(frames - i), seed)); // 60 ms gap
                i += 2;
            }
        }
        out
    }

    fn vad() -> EnergyVad {
        let mut v = EnergyVad::new(&SttConfig::default());
        v.debug_energy = false;
        v
    }

    /// Feed audio in uneven callback-sized chunks, like a real device.
    /// Returns (utterances, frames fed when each ended).
    fn feed(v: &mut EnergyVad, audio: &[f32]) -> Vec<(Vec<f32>, usize)> {
        let mut out = Vec::new();
        let mut fed = 0;
        for chunk in audio.chunks(1024) {
            fed += chunk.len();
            if let Some(u) = v.process(chunk) {
                out.push((u, fed));
            }
        }
        // Flush anything left pending by a mid-callback end.
        if let Some(u) = v.process(&[]) {
            out.push((u, fed));
        }
        out
    }

    #[test]
    fn quiet_room_behaves_as_before() {
        let mut s = 1;
        let mut v = vad();
        let mut audio = noise(0.033, 60, &mut s);
        audio.extend(speech(0.2, 0.033, 1500, &mut s));
        audio.extend(noise(0.033, 60, &mut s));
        let got = feed(&mut v, &audio);
        assert_eq!(got.len(), 1, "exactly one utterance");
        assert!((v.start_threshold() - SPEECH_START_MIN).abs() < 1e-6);
        assert!((v.end_threshold() - SPEECH_END_MIN).abs() < 1e-6);
    }

    #[test]
    fn noisy_room_still_ends_promptly() {
        // Ambient at 0.09 — above the old fixed 0.075 start and 0.06 end,
        // the situation in the 2026-09-22 log.
        let mut s = 2;
        let mut v = vad();
        let mut audio = noise(0.09, 60, &mut s);
        let speech_start = audio.len();
        audio.extend(speech(0.4, 0.09, 1500, &mut s));
        let speech_end = audio.len();
        audio.extend(noise(0.09, 300, &mut s)); // 9 s of room
        let got = feed(&mut v, &audio);
        assert_eq!(got.len(), 1, "one utterance, no false starts: {:?}", got.len());
        let ended_at = got[0].1;
        assert!(ended_at > speech_start);
        // Ends within ~1 s of the user stopping (500 ms pause + smoothing
        // + callback granularity), not at a ceiling.
        assert!(
            ended_at - speech_end < 16000,
            "ended {} samples after speech",
            ended_at - speech_end
        );
    }

    #[test]
    fn room_getting_louder_mid_utterance_does_not_hold_it_open() {
        // Quiet room, user speaks, then fans spin up to 0.1 — above the
        // end threshold learned while it was quiet.
        let mut s = 3;
        let mut v = vad();
        let mut audio = noise(0.03, 60, &mut s);
        audio.extend(speech(0.3, 0.03, 1500, &mut s));
        let speech_end = audio.len();
        audio.extend(noise(0.10, 1500, &mut s)); // 45 s of fan
        let got = feed(&mut v, &audio);
        assert!(!got.is_empty(), "utterance must end");
        let (utt, ended_at) = &got[0];
        assert!(
            ended_at - speech_end < 16000 * 5,
            "took {:.1}s after speech to end",
            (ended_at - speech_end) as f32 / 16000.0
        );
        // And the fan noise tail was trimmed off: roughly the 1.5 s of
        // speech plus pre-roll and padding, not 5 s of it.
        assert!(utt.len() < 16000 * 3, "utterance {} samples", utt.len());
        // No further utterances manufactured from the fan.
        assert_eq!(got.len(), 1, "fan noise produced {} extra utterances", got.len() - 1);
    }

    #[test]
    fn noise_floor_survives_reset() {
        let mut s = 4;
        let mut v = vad();
        feed(&mut v, &noise(0.09, 80, &mut s));
        let learned = v.noise_floor;
        assert!(learned > 0.07, "learned {}", learned);
        v.reset();
        assert_eq!(v.noise_floor, learned);
        // Resuming in the same room (as after MAVIS speaks) must not
        // trigger on the room itself.
        let got = feed(&mut v, &noise(0.09, 200, &mut s));
        assert!(got.is_empty());
        assert!(!v.is_speaking);
    }

    #[test]
    fn partial_frames_are_carried_not_lost() {
        let mut s = 5;
        let mut v = vad();
        let audio = noise(0.03, 40, &mut s);
        // 7-sample callbacks: none is a whole frame.
        for c in audio.chunks(7) {
            v.process(c);
        }
        assert_eq!(v.calibration_left, 0, "every frame was analysed");
        assert!(v.pending.len() < FRAME);
    }
}