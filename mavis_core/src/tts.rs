//! TTS module — functionality moved into Executor::run_say() for blocking
//! playback with proper STT mute coordination. The old TtsEngine here was
//! fire-and-forget (spawned aplay and returned immediately) and never
//! emitted UiStateChange, so the STT mute controller never actually
//! muted the mic during playback. Kept as a stub, not deleted, so
//! `mod tts;` in main.rs stays valid and this history isn't silently lost.