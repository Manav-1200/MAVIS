use std::process::{Command, Stdio};
use std::io::Write;
use log::{info, warn, error, debug};

/// Neural TTS engine using Piper (local, offline).
/// Falls back to speech-dispatcher (spd-say) if Piper is unavailable.
pub struct TtsEngine {
    piper_available: bool,
    voice_model_path: String,
}

impl TtsEngine {
    pub fn new() -> Self {
        // Allow voice selection via env var. Defaults try more natural voices first.
        let voice_model_path = std::env::var("MAVIS_VOICE_MODEL")
            .unwrap_or_else(|_| {
                let home = std::env::var("HOME").unwrap_or_default();
                let candidates = [
                    // lessac is significantly more natural than amy
                    format!("{}/.local/share/piper-voices/en_US-lessac-medium.onnx", home),
                    format!("{}/.local/share/piper-voices/en_US-lessac-high.onnx", home),
                    format!("{}/.local/share/piper-voices/en_US-ryan-medium.onnx", home),
                    format!("{}/.local/share/piper-voices/en_US-ryan-high.onnx", home),
                    // fallback to existing amy voice
                    format!("{}/.local/share/piper-voices/en_US-amy-medium.onnx", home),
                ];
                for c in &candidates {
                    if std::path::Path::new(c).exists() {
                        return c.clone();
                    }
                }
                candidates[0].clone() // default even if missing
            });

        let piper_available = Command::new("which")
            .arg("piper")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        if piper_available {
            if std::path::Path::new(&voice_model_path).exists() {
                info!("TTS: Piper ready with voice {}", voice_model_path);
            } else {
                warn!(
                    "Piper binary found but voice model missing at {}. \
                     Download a voice from https://github.com/rhasspy/piper/releases",
                    voice_model_path
                );
            }
        } else {
            info!("TTS: Piper not found, using spd-say fallback");
        }

        Self {
            piper_available,
            voice_model_path,
        }
    }

    /// Speak the given text. Fire-and-forget: non-blocking, returns immediately.
    /// Text is truncated to 500 chars to avoid engine overload.
    pub fn say(&self, text: &str) {
        let text = if text.len() > 500 {
            &text[..500]
        } else {
            text
        };

        if self.piper_available && std::path::Path::new(&self.voice_model_path).exists() {
            debug!("TTS (Piper): {}", text);

            let mut piper = match Command::new("piper")
                .args(&[
                    "--model", &self.voice_model_path,
                    "--output_file", "-",
                    "--length-scale", "1.15",      // Slightly slower = more natural, less robotic
                    "--sentence-silence", "0.25",  // Natural pauses between sentences
                ])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
            {
                Ok(child) => child,
                Err(e) => {
                    error!("TTS: Failed to spawn Piper: {}", e);
                    self.fallback_say(text);
                    return;
                }
            };

            if let Some(mut stdin) = piper.stdin.take() {
                if let Err(e) = stdin.write_all(text.as_bytes()) {
                    error!("TTS: Failed to write to Piper stdin: {}", e);
                    self.fallback_say(text);
                    return;
                }
                // stdin drops here, closing the pipe so Piper processes the text
            }

            let _ = Command::new("aplay")
                .args(&["-r", "22050", "-f", "S16_LE", "-c", "1", "-t", "raw", "-"])
                .stdin(piper.stdout.take().unwrap())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
        } else {
            self.fallback_say(text);
        }
    }

    fn fallback_say(&self, text: &str) {
        warn!("TTS: spd-say fallback for: {}", text);
        let _ = Command::new("spd-say")
            .arg(text)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }
}