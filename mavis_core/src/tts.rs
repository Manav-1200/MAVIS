use std::process::Stdio;
use log::{info, warn, error, debug};
use tokio::process::Command;
use tokio::io::AsyncWriteExt;

/// Neural TTS engine using Piper (local, offline).
/// Falls back to speech-dispatcher (spd-say) if Piper is unavailable.
pub struct TtsEngine {
    piper_available: bool,
    voice_model_path: String,
}

impl TtsEngine {
    pub fn new() -> Self {
        let voice_model_path = format!(
            "{}/.local/share/piper-voices/en_US-amy-medium.onnx",
            std::env::var("HOME").unwrap_or_default()
        );

        let piper_available = std::process::Command::new("which")
            .arg("piper")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        if piper_available {
            if std::path::Path::new(&voice_model_path).exists() {
                info!("TTS: Piper ready with voice {}", voice_model_path);
            } else {
                warn!(
                    "Piper binary found but voice model missing at {}",
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

    /// Speak the given text. Blocks until audio playback finishes.
    /// Text is truncated to 500 chars to avoid engine overload.
    pub async fn say(&self, text: &str) -> Result<(), anyhow::Error> {
        let text = if text.len() > 500 {
            &text[..500]
        } else {
            text
        };

        if self.piper_available && std::path::Path::new(&self.voice_model_path).exists() {
            debug!("TTS (Piper): {}", text);

            let mut piper = match Command::new("piper")
                .args(&["--model", &self.voice_model_path, "--output_file", "-"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
            {
                Ok(child) => child,
                Err(e) => {
                    error!("TTS: Failed to spawn Piper: {}", e);
                    self.fallback_say(text).await;
                    return Ok(());
                }
            };

            if let Some(mut stdin) = piper.stdin.take() {
                if let Err(e) = stdin.write_all(text.as_bytes()).await {
                    error!("TTS: Failed to write to Piper stdin: {}", e);
                    self.fallback_say(text).await;
                    return Ok(());
                }
                // stdin drops here, closing the pipe so Piper processes the text
            }

            let mut aplay = match Command::new("aplay")
                .args(&["-r", "22050", "-f", "S16_LE", "-c", "1", "-t", "raw", "-"])
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            {
                Ok(child) => child,
                Err(e) => {
                    error!("TTS: Failed to spawn aplay: {}", e);
                    return Ok(());
                }
            };

            // Pipe piper stdout -> aplay stdin asynchronously
            let copy_handle = if let (Some(mut piper_out), Some(mut aplay_in)) =
                (piper.stdout.take(), aplay.stdin.take())
            {
                Some(tokio::spawn(async move {
                    let _ = tokio::io::copy(&mut piper_out, &mut aplay_in).await;
                }))
            } else {
                None
            };

            let _ = piper.wait().await;
            if let Some(h) = copy_handle {
                let _ = h.await;
            }
            let _ = aplay.wait().await;

        } else {
            self.fallback_say(text).await;
        }
        Ok(())
    }

    async fn fallback_say(&self, text: &str) {
        warn!("TTS: spd-say fallback for: {}", text);
        let _ = Command::new("spd-say")
            .arg(text)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }
}