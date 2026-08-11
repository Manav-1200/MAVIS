//! Neural TTS via Piper (local, offline, CPU-fast).
//! Falls back to spd-say if Piper or its voice model is missing.

use std::path::Path;
use std::process::{Command, Stdio};
use log::{debug, error, info, warn};

pub struct TtsEngine {
    model_path: String,
    config_path: String,
    use_piper: bool,
}

impl TtsEngine {
    pub fn new() -> Self {
        let model_path = std::env::var("HOME")
            .map(|h| format!("{}/.local/share/piper-voices/en_US-lessac-medium.onnx", h))
            .unwrap_or_default();

        let config_path = format!("{}.json", model_path);

        let piper_in_path = Command::new("sh")
            .args(["-c", "command -v piper"])
            .stdout(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);

        let model_exists = Path::new(&model_path).exists();
        let config_exists = Path::new(&config_path).exists();

        let use_piper = piper_in_path && model_exists && config_exists;

        if use_piper {
            info!("TTS: Piper ready ({})", model_path);
        } else {
            warn!(
                "TTS: Piper unavailable (binary={}, model={}, config={}). Falling back to spd-say.",
                piper_in_path, model_exists, config_exists
            );
        }

        Self {
            model_path,
            config_path,
            use_piper,
        }
    }

    /// Speak the given text. Non-blocking; returns immediately.
    pub fn say(&self, text: &str) {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            warn!("TTS received empty text, skipping");
            return;
        }

        const MAX_LEN: usize = 500;
        let text = if trimmed.len() > MAX_LEN {
            warn!("TTS text too long ({} chars), truncating", trimmed.len());
            &trimmed[..MAX_LEN]
        } else {
            trimmed
        };

        if self.use_piper {
            self.say_piper(text);
        } else {
            self.say_spd(text);
        }
    }

    fn say_piper(&self, text: &str) {
        debug!("TTS (Piper): {}", text);

        // Piper outputs raw 22050Hz mono S16LE PCM on stdout when --output_file is -
        // Pipe directly to aplay with correct format flags.
        let safe_text = text.replace("'", "'\\''");

        let cmd = format!(
            "printf '%s' '{}' | piper --model '{}' --config '{}' --output_file - | aplay -r 22050 -f S16_LE -c 1 -t raw -",
            safe_text, self.model_path, self.config_path
        );

        match Command::new("sh")
            .arg("-c")
            .arg(&cmd)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(_) => info!("TTS spawned (Piper)"),
            Err(e) => {
                error!("Piper TTS failed ({}), falling back to spd-say", e);
                self.say_spd(text);
            }
        }
    }

    fn say_spd(&self, text: &str) {
        debug!("TTS (spd-say fallback): {}", text);

        match Command::new("spd-say")
            .arg(text)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(_) => info!("TTS spawned (spd-say fallback)"),
            Err(e) => error!("spd-say also failed: {}", e),
        }
    }
}

impl Default for TtsEngine {
    fn default() -> Self {
        Self::new()
    }
}