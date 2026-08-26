//! MAVIS STT Manager — Rust runtime side

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, SampleFormat, SampleRate, SupportedStreamConfig};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
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
    pub max_utterance_duration_ms: u64,
    pub min_max_energy: f32,
}

impl Default for SttConfig {
    fn default() -> Self {
        Self {
            sample_rate: 16000,
            silence_duration_ms: 600,
            min_speech_duration_ms: 250,
            frame_duration_ms: 30,
            max_utterance_duration_ms: 15000,
            min_max_energy: 0.04,
        }
    }
}

// ---------------------------------------------------------------------------
// VAD — adaptive energy threshold with noise-floor tracking
// ---------------------------------------------------------------------------

struct EnergyVad {
    frame_size: usize,
    silence_threshold_frames: usize,
    min_speech_threshold_frames: usize,
    max_speech_frames: usize,
    base_threshold: f32,
    noise_floor: f32,
    buffer: VecDeque<f32>,
    speech_frames: usize,
    silence_frames: usize,
    pub is_speaking: bool,
    max_energy_seen: f32,
    pub last_max_energy: f32,
    sample_rate: usize,
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
            max_speech_frames: ((cfg.max_utterance_duration_ms / cfg.frame_duration_ms).max(1))
                as usize,
            base_threshold: 0.02,
            noise_floor: 0.005,
            buffer: VecDeque::new(),
            speech_frames: 0,
            silence_frames: 0,
            is_speaking: false,
            max_energy_seen: 0.0,
            last_max_energy: 0.0,
            sample_rate: cfg.sample_rate as usize,
        }
    }

    fn effective_threshold(&self) -> f32 {
        self.base_threshold
            .max(self.noise_floor * 1.5)
            .min(0.022)
    }

    fn process(&mut self, samples: &[f32]) -> Option<Vec<f32>> {
        for chunk in samples.chunks(self.frame_size) {
            if chunk.len() < self.frame_size {
                self.buffer.extend(chunk);
                continue;
            }

            let energy =
                (chunk.iter().map(|s| s * s).sum::<f32>() / chunk.len() as f32).sqrt();
            self.max_energy_seen = self.max_energy_seen.max(energy);
            let threshold = self.effective_threshold();

            self.buffer.extend(chunk);

            if energy > threshold {
                self.speech_frames += 1;
                self.silence_frames = 0;
                if self.speech_frames >= self.min_speech_threshold_frames {
                    if !self.is_speaking {
                        info!(
                            "VAD: SPEECH START (energy={:.4}, threshold={:.4}, noise_floor={:.4})",
                            energy, threshold, self.noise_floor
                        );
                        self.is_speaking = true;
                    }
                }
            } else {
                self.noise_floor = (self.noise_floor * 0.97 + energy * 0.03).min(0.015);

                if self.is_speaking {
                    self.silence_frames += 1;
                    if self.silence_frames >= self.silence_threshold_frames {
                        let utterance: Vec<f32> = self.buffer.drain(..).collect();
                        self.last_max_energy = self.max_energy_seen;
                        info!(
                            "VAD: SPEECH END ({} samples, {} frames, max_energy={:.3}, noise_floor={:.4})",
                            utterance.len(),
                            self.speech_frames,
                            self.max_energy_seen,
                            self.noise_floor
                        );
                        self.reset();
                        return Some(utterance);
                    }
                } else {
                    while self.buffer.len() > self.sample_rate {
                        self.buffer.pop_front();
                    }
                    self.speech_frames = self.speech_frames.saturating_sub(1);
                }
            }

            if self.is_speaking && self.speech_frames >= self.max_speech_frames {
                let utterance: Vec<f32> = self.buffer.drain(..).collect();
                self.last_max_energy = self.max_energy_seen;
                info!(
                    "VAD: FORCED END ({} samples, {} frames, max_energy={:.3}, noise_floor={:.4})",
                    utterance.len(),
                    self.speech_frames,
                    self.max_energy_seen,
                    self.noise_floor
                );
                self.reset();
                return Some(utterance);
            }
        }
        None
    }

    fn reset(&mut self) {
        self.buffer.clear();
        self.speech_frames = 0;
        self.silence_frames = 0;
        self.is_speaking = false;
        self.max_energy_seen = 0.0;
        self.noise_floor = 0.005;
    }
}

// ---------------------------------------------------------------------------
// Handle
// ---------------------------------------------------------------------------

pub struct SttHandle {
    running: Arc<AtomicBool>,
    pub tts_active: Arc<AtomicBool>,
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
        let s = score(&name) as i32;
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

    /// Start the STT pipeline.
    ///
    /// Returns:
    /// - `SttHandle` — call `.stop()` to shut down the audio stream
    /// - `mpsc::Receiver<Vec<f32>>` — utterance audio chunks for the worker
    /// - `mpsc::Receiver<f32>` — real-time per-frame RMS energy for the orb LED
    pub fn start(self, bus: Arc<EventBus>, speech_start_tx: Option<mpsc::Sender<()>>) -> (SttHandle, mpsc::Receiver<Vec<f32>>, mpsc::Receiver<f32>) {
        let running = Arc::new(AtomicBool::new(true));
        let running_stream = running.clone();
        let tts_active = Arc::new(AtomicBool::new(false));
        let tts_active_stream = tts_active.clone();

        let (tx, rx) = mpsc::channel::<Vec<f32>>(4);
        let (energy_tx, energy_rx) = mpsc::channel::<f32>(64);
        let vad = Arc::new(Mutex::new(EnergyVad::new(&self.config)));
        let config = self.config.clone();

        let host = cpal::default_host();

        let device = select_input_device(&host)
            .expect("No input device available");

        let mut supported_configs = device
            .supported_input_configs()
            .expect("Error querying input configs");

        let stream_config: SupportedStreamConfig = supported_configs
            .find(|c| {
                c.sample_format() == SampleFormat::F32
                    && c.min_sample_rate() <= SampleRate(16000)
                    && c.max_sample_rate() >= SampleRate(16000)
            })
            .map(|c| c.with_sample_rate(SampleRate(16000)))
            .or_else(|| {
                let mut configs = device.supported_input_configs().ok()?;
                configs
                    .find(|c| c.sample_format() == SampleFormat::F32)
                    .map(|c| c.with_max_sample_rate())
            })
            .or_else(|| {
                let mut configs = device.supported_input_configs().ok()?;
                configs.next().map(|c| c.with_max_sample_rate())
            })
            .expect("No supported input config");

        info!("STT stream config: {:?}", stream_config);

        let sample_rate = stream_config.sample_rate().0;
        let channels = stream_config.channels() as usize;
        let sample_format = stream_config.sample_format();

        let err_fn = |err| error!("STT stream error: {}", err);

        let stream = match sample_format {
            SampleFormat::F32 => {
                let vad = vad.clone();
                let tx = tx.clone();
                let energy_tx = energy_tx.clone();
                let bus_for_ui = bus.clone();
                let speech_start = speech_start_tx.clone();
                device
                    .build_input_stream(
                        &stream_config.into(),
                        move |data: &[f32], _: &cpal::InputCallbackInfo| {
                            if !running_stream.load(Ordering::Relaxed) {
                                return;
                            }
                            if tts_active_stream.load(Ordering::Relaxed) {
                                return;
                            }

                            let mono: Vec<f32> = if channels == 1 {
                                data.to_vec()
                            } else {
                                data.chunks(channels)
                                    .map(|c| c.iter().sum::<f32>() / channels as f32)
                                    .collect()
                            };

                            let target = config.sample_rate as usize;
                            let resampled = if sample_rate as usize == target {
                                mono
                            } else {
                                resample_linear(&mono, sample_rate, target as u32)
                            };

                            // Real-time energy for voice activity LED
                            if !resampled.is_empty() {
                                let frame_energy = resampled.iter().map(|s| s * s).sum::<f32>() / resampled.len() as f32;
                                let _ = energy_tx.try_send(frame_energy);
                            }

                            let mut vad_guard = vad.lock().unwrap();
                            let was_speaking = vad_guard.is_speaking;
                            let result = vad_guard.process(&resampled);
                            let max_energy = vad_guard.last_max_energy;
                            let now_speaking = vad_guard.is_speaking;
                            drop(vad_guard);

                            if let Some(utterance) = result {
                                if max_energy < config.min_max_energy {
                                    info!(
                                        "STT: dropping noise utterance (max_energy={:.3} < {:.3})",
                                        max_energy, config.min_max_energy
                                    );
                                    let _ = bus_for_ui.publish(Event {
                                        id: uuid::Uuid::new_v4(),
                                        timestamp: chrono::Utc::now(),
                                        source: "stt".to_string(),
                                        event_type: EventType::UiStateChange,
                                        payload: serde_json::json!({ "state": "idle" }),
                                    });
                                    return;
                                }

                                info!("STT: shipping utterance ({} samples)", utterance.len());
                                let _ = tx.try_send(utterance);
                                let _ = bus_for_ui.publish(Event {
                                    id: uuid::Uuid::new_v4(),
                                    timestamp: chrono::Utc::now(),
                                    source: "stt".to_string(),
                                    event_type: EventType::UiStateChange,
                                    payload: serde_json::json!({ "state": "thinking" }),
                                });
                            } else if !was_speaking && now_speaking {
                                if let Some(ref sstx) = speech_start {
                                    let _ = sstx.try_send(());
                                }
                                let _ = bus_for_ui.publish(Event {
                                    id: uuid::Uuid::new_v4(),
                                    timestamp: chrono::Utc::now(),
                                    source: "stt".to_string(),
                                    event_type: EventType::UiStateChange,
                                    payload: serde_json::json!({ "state": "listening" }),
                                });
                            }
                        },
                        err_fn,
                        None,
                    )
                    .expect("Failed to build input stream")
            }
            _ => panic!("Unsupported sample format: {:?}", sample_format),
        };

        stream.play().expect("Failed to start audio stream");
        info!("STT listening active. Speak for 1-2 seconds, then pause.");

        let handle = SttHandle { running, tts_active, _stream: stream };
        (handle, rx, energy_rx)
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