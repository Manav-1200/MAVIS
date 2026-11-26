<!-- PHASES.md -->
# MAVIS — Project Phases

> **Tagline:** A persistent desktop-native AI companion. Not a chatbot.
>
> **Project goal:** Build a local-first, privacy-first desktop AI companion. Rust owns the runtime (~50 MB, always on). Python owns the AI (spawned on demand, killed when idle).
>
> **Repo:** `github.com/Manav-1200/MAVIS`
>
> **Stack:** Rust 1.85+ (tokio, minifb, serde, rusqlite, notify, cpal), Python 3.10+ (llama-cpp-python, faster-whisper, kokoro, piper as fallback)
>
> **Why things are the way they are:** see [`DECISIONS.md`](DECISIONS.md) — every decision, the problem behind it, and the evidence for each fix.

---

## Quick reference — phase overview

| Phase | Name | Core deliverable | Portfolio label | Status |
|-------|------|-----------------|-----------------|--------|
| 1 | Foundation | Rust runtime compiles. Async event bus. Orb window appears. Python worker reorganized. | Rust desktop app | :white_check_mark: Complete |
| 2 | Core Runtime | Context engine. Layered memory (SQLite). System integration. Orb states. Graceful shutdown. | Event-driven runtime | :white_check_mark: Complete |
| 3 | AI Worker | Rust <-> Python bridge. Local LLM loads. First inference. Worker lifecycle. | Local AI pipeline | :white_check_mark: Complete |
| 4 | Integration | Voice wake -> STT -> LLM -> TTS. Intent system. First automations. | Full voice companion | :white_check_mark: Complete |
| 5 | Interaction Polish | TTS queue, interruption, session recovery, personality foundation. | Daily polish | :white_check_mark: Complete |
| 6 | Context Awareness | Active window, open windows, workspace, clipboard, IDE, terminal, project, calendar. | Companion senses | :white_check_mark: Complete |
| 6.5 | Action Execution | App launching, media, web search, system control; cross-platform app discovery. | Companion acts | :white_check_mark: Built |
| 7 | Memory & Learning | Recall with decay, daily consolidation, episodic replay, entity graph, context compression. | Persistent memory | :white_check_mark: 7.1–7.2 complete; 7.3 deferred |
| 8 | Safety & Permissions | Risk scoring, permission gate, confirmation flow, rollback, append-only audit log. | Trust layer | :white_check_mark: Built (per-skill permissions wait on Phase 9) |
| 8.5 | System Sentinel | Notices what changed on the machine: packages, privileges, file integrity, advisories. | Machine awareness | :construction: Built — steps 2b–4 await a hardware run, step 5 unverified |
| 8.6 | Understanding & Trust | The model decides what you mean (DO/SAY); sensitive actions warn and ask; your voice and face authorise them; it learns how you talk; it reads files and builds projects. | Understands, not scripted | :construction: In progress |
| 9 | Skills Platform | Plugin API with manifest, lifecycle hooks, sandboxing, core skills. | Extensible companion | Not started |
| 10 | Automation & Proactive Intelligence | Rule engine, predictive suggestions, workflow recording, wellness reminders, daily briefing. | Proactive assistant | Not started |
| 11 | Vision & Advanced UX | OCR, screenshot understanding, UI element detection, secondary monitor dashboard, conversation history. | Sees the screen | Not started |
| 12 | Multi-Model & Cross-Platform | Local model routing, benchmarking, Linux/Windows/macOS abstraction. | Runs everywhere | Not started |

---

## Architecture

```
+-------------+     +-------------+
| Living Orb  |---->|   Context   |
|   (Rust)    |     |   Engine    |
+-------------+     |   (Rust)    |
       ^            +------+------+
       |                   |
       |            +------v------+        +-------------+
       |            |   Planner   |<------>|  AI Worker  |
       |            |   (Rust)    |  UDS   |  (Python)   |
       |            +------+------+        +-------------+
       |                   | PlanReady
       |            +------v------+
       |            | Permission  |---> audit.db
       |            |    Gate     |
       |            +------+------+
       |                   | PlanApproved
       |            +------v------+
       +------------|   Executor  |
                    |   (Rust)    |
                    +-------------+

  Sentinel (Rust) --SystemChange--> Context Engine
```

Everything above communicates over the in-process event bus (`tokio::sync::broadcast`). Only the Python worker is reached over a socket.

**Runtime split:**
- **`mavis_core` (Rust):** UI, event bus, context engine, memory, system integration. ~50 MB. Always on.
- **`mavis_worker` (Python):** AI inference, model weights, voice. Spawned on demand. Killed when idle.

**Protocol:** subsystems talk over an in-process event bus. The Python worker is reached over a Unix domain socket carrying length-prefixed JSON. Not HTTP. Not gRPC.

**Supervision:** every event-loop subsystem runs under `util::supervise`. A panic is logged and the subsystem rebuilt and restarted, instead of silently disappearing.

---

## Phase 1 — Foundation

**Status:** :white_check_mark: COMPLETE (2026-08-06)

**Goal:** `mavis_core` compiles and runs. The event bus works. An orb window appears. The Python worker is reorganized.

### 1.1 — Project scaffold
- [x] Workspace structure: `mavis_core/` (Rust), `mavis_worker/` (Python), `config/`, `data/`, `memory/`, `logs/`
- [x] `uv` for Python, `ruff` for format/lint, `pre-commit` for git hooks
- [x] `mavis_core/Cargo.toml` — tokio, serde, serde_json, uuid, chrono, rusqlite, dirs, log, env_logger, anyhow, thiserror, minifb
- [x] `mavis_worker/pyproject.toml` — hatchling backend, package layout

### 1.2 — Event Bus
- [x] `models/event.rs` — `Event` struct + `EventType` enum (serde, PartialEq)
- [x] `event_bus.rs` — `tokio::sync::broadcast`, capacity 256, `close()` for graceful shutdown
- [x] Methods: `new()`, `publish()`, `subscribe()`, `close()`
- [x] Unit tests pass

### 1.3 — Entry point
- [x] `main.rs` — tokio async main, subsystem wiring
- [x] Init logging. Create event bus. Spawn all subsystems.
- [x] `Ctrl+C` triggers `bus.close()` -> all receivers wake, concurrent join with 5s timeout
- [x] Clean shutdown under 1 second

### 1.4 — Orb UI
- [x] `ui/orb.rs` — `Orb` struct, dedicated render thread
- [x] `ui/states.rs` — `OrbState` enum: Idle, Listening, Thinking, Speaking, Working, Error, Asleep
- [x] 80x80 px, borderless, transparent, always-on-top
- [x] Draggable (click-and-drag to reposition) *— did not actually work until 2026-09-16: `MouseMode::Clamp` confined cursor readings to the orb's own 80×80 area. Fixed with `MouseMode::Pass`. `MAVIS_ORB_POS=x,y` sets the start position; native Wayland ignores it, and `wlr-layer-shell` is the real fix (not built).*
- [x] Soft pulsing circle with smoothstep gradient, state-reactive colors
- [x] **Decision:** `minifb` for Phase 1-2. Raw Wayland in Phase 6.

### 1.5 — Subsystem stubs
- [x] `context_engine.rs` — stub with `new()` and `process_event()`
- [x] `planner.rs` — stub with `run()` signature
- [x] `executor.rs` — stub with `run()` signature
- [x] `worker_bridge.rs` — UDS client stub
- [x] `memory/manager.rs` — `MemoryManager` facade
- [x] `memory/working.rs` — in-memory store
- [x] `memory/permanent.rs`, `episodic.rs` — SQLite stubs *(`permanent.rs` removed 2026-09-19, `episodic.rs` 2026-10-08 — neither was ever read)*
- [x] `memory/long_term.rs`, `session.rs` — empty stubs *(`long_term.rs` made real in Phase 7; `session.rs` removed 2026-09-19)*
- [x] `system/dbus.rs`, `hotkeys.rs`, `watcher.rs` — empty stubs

### 1.6 — Python worker reorganization
- [x] `core/config.py` — TOML loading, validation, dot-notation `get()`
- [x] `core/logger.py` — logging setup
- [x] `core/events.py` — in-process EventBus for Python side *(removed 2026-09-14 — nothing used it)*
- [x] `bootstrap.py` — creates dirs, loads config, starts `MavisApp` *(removed 2026-09-14 along with `app.py`, `main.py` and `core/lifecycle.py`; the worker starts from `worker.py`)*
- [x] `worker.py` — asyncio UDS server at `/tmp/mavis_worker.sock`, lazy AI imports, responds with `[STUB]`

### 1.7 — Phase 1 wrap-up
- [x] `cargo check` passes clean
- [x] `cargo run` opens orb window, stays alive until `Ctrl+C`
- [x] `cargo test` passes for event bus and models
- [x] `pip install -e ./mavis_worker` succeeds
- [x] `python -m mavis.worker` starts UDS server
- [ ] Tag: `v0.1.0-foundation` — *never created; `v0.3.0-ai-worker` is the only tag*

---

## Phase 2 — Core Runtime

**Status:** :white_check_mark: COMPLETE (2026-08-06)

**Goal:** Context engine routes events. Memory persists to SQLite. System events flow. Orb shows real state transitions. Executor runs actions.

### 2.1 — Context Engine
- [x] Maintains `WorkingMemory` (50-event context window)
- [x] `process_event()` routes all `EventType` variants
- [x] Auto-persists `UserIntent`, `ActionComplete`, `PlanReady` to EpisodicStore *(removed 2026-10-08: written, never read; recall.db covers replay)*
- [x] Routes `WorkerResponse` -> `ContextUpdate` or passes to Planner
- [x] **Fix:** Removed double-publish of `PlanReady` (was causing duplicate execution)

### 2.2 — Memory layers
- [x] `PermanentStore` — SQLite key-value: `get/set/delete/list`
- [x] `EpisodicStore` — SQLite event log: `record/recent/search`
- [x] `WorkingMemory` — in-memory `VecDeque<Event>` with intent/plan/state tracking
- [x] `LongTermMemory` / `SessionStore` — stubs (Phase 6)
- [x] `MemoryManager` — owns all layers behind `RwLock`/`Mutex`

### 2.3 — Planner
- [x] Listens for `UserIntent` -> generates `WorkerRequest`
- [x] Listens for `WorkerResponse` (type "plan") -> emits `PlanReady`
- [x] Never executes. Returns plan to Executor via Event Bus.

### 2.4 — Executor
- [x] Receives `PlanReady` -> parses flexible payloads (array, object with `actions`, single action)
- [x] Action types: `shell`, `app`, `notify`, `say`, `system`
- [x] `shell` -> `sh -c` with stdout/stderr capture
- [x] `app` -> `Command::spawn` with `xdg-open` fallback for URLs/paths
- [x] `notify` -> `notify-send`
- [x] `say` -> `spd-say` / `espeak` fallback / log-only
- [x] `system` -> delegates to `SystemAction` event for DBusIntegration
- [x] Emits `ActionComplete` after each action
- [x] Emits `UiStateChange` (working -> idle/error) for Orb feedback
- [x] Stops plan execution on first failure

### 2.5 — System integration
- [x] `system/dbus.rs` — `DbusIntegration`: listens for `SystemAction` events
  - `notify` -> `notify-send`
  - `media_play_pause/next/previous` -> `playerctl` / `dbus-send` fallback
  - `brightness_up/down` -> `brightnessctl`, or systemd-logind's `SetBrightness` when it isn't installed (6.23)
  - `volume_up/down/mute` -> `pactl`
- [x] `system/hotkeys.rs` — `HotkeyManager`: UDS socket at `/tmp/mavis_hotkey.sock`
  - Wayland-compatible (no X11 grab needed)
  - Emits `UserIntent` with `trigger: "hotkey"`
  - `tokio::select!` for graceful shutdown
- [x] `system/watcher.rs` — `FileWatcher`: watches `~/Downloads` and `~/Desktop`
  - Uses `notify` crate (inotify on Linux)
  - Emits `ContextUpdate` on file changes

### 2.6 — Orb state machine
- [x] All 7 states implemented with distinct colors and pulse frequencies
- [x] Transitions driven by `UiStateChange` events from Executor
- [x] "Never intrusive" — small, draggable, transparent background

### 2.7 — Error handling
- [x] Subsystem isolation — one crash cannot kill runtime
- [x] `anyhow` for app errors, `thiserror` for library errors
- [x] Executor logs failures, emits `ActionComplete` with `success: false`

### 2.8 — Phase 2 wrap-up
- [x] Hotkey socket accepts connections and emits intents
- [x] File changes appear in working memory via `ContextUpdate`
- [x] Memory persists across restarts (SQLite)
- [x] Planner decomposes intent -> Executor runs actions
- [x] `cargo test` passes (7 tests)
- [x] Graceful shutdown under 1 second
- [ ] Tag: `v0.2.0-core-runtime` — *never created*

---

## Phase 3 — AI Worker

**Status:** :white_check_mark: COMPLETE (2026-08-10)

**Goal:** Replace `[STUB]` with real model loading. Local quantized LLM in 6GB VRAM. First inference. Worker lifecycle managed.

### 3.1 — Rust–Python bridge
- [x] JSON over UDS (Unix domain socket). Not HTTP. Not gRPC.
- [x] `worker_bridge.rs` — `WorkerBridge` struct: `run()`, forwards `WorkerRequest`, publishes `WorkerResponse`
- [x] Python side — `worker.py` reads UDS, parses JSON, routes to inference, writes response
- [x] Timeout handling on requests
- [x] Health check / heartbeat

### 3.2 — Worker lifecycle
- [x] Eager spawn — worker starts with runtime, socket ready immediately
- [x] Health check every 30s. Restart if dead.
- [x] Idle timeout — request model unload after 5 min. Reclaim VRAM.
- [x] Crash recovery — restart once. Second crash within 60s -> mark unavailable.
- [x] `MAVIS_PYTHON_PATH` env var for venv python

### 3.3 — Model loading
- [x] **Decision:** `llama-cpp-python` with quantized models (Q4_K_M)
- [x] Target: 3-4 GB model, 2-3 GB headroom on RTX 4050 6GB
- [x] Primary: `Phi-3-mini-4k-instruct-Q4_K_M.gguf` (~2.3 GB)
- [x] `mavis_worker/src/mavis/inference/engine.py` — `load_model()`, `generate()`, `chat()`, `get_memory_usage()`
- [x] First inference: "Hello MAVIS" -> load -> generate -> orb `Thinking -> Speaking`

### 3.4 — Prompt system
- [x] `mavis_worker/src/mavis/inference/prompts.py` — prompt templates
- [x] System prompt defines MAVIS personality
- [x] `build_chat_messages()` with working memory injection support
- [x] Context injection from working memory

### 3.5 — Phase 3 wrap-up
- [x] First request -> worker spawns -> model loads -> response -> orb animates. Under 10s.
- [x] Worker unloads after idle. VRAM reclaimed.
- [x] Crash recovery works.
- [x] Tag: `v0.3.0-ai-worker`

---

## Phase 4 — Integration

**Status:** :white_check_mark: COMPLETE (2026-08-26)

**Goal:** Voice in, voice out. Intent classification. First automations.

### 4.1 — Speech-to-text
- [x] **Decision:** `faster-whisper` (not whisper.cpp). CPU int8 to avoid VRAM contention.
- [x] Audio capture via `cpal`. Energy-based VAD for speech segmentation.
- [x] Explicit ALSA device selection (`sysdefault:CARD=Generic`) to avoid silent "default" route.
- [x] `vad_filter=False` in faster-whisper — Rust VAD is single source of truth. *(Reversed 2026-09-22: Silero now gates what Whisper hears — see §6.4.)*
- [x] `min_speech_duration_ms=500` to filter noise bursts.
- [x] UDS protocol: `readexactly()` fix for partial socket reads on large audio payloads.
- [x] STT request timeout with retries (model load can be slow).

### 4.2 — Text-to-speech
- [x] **Decision:** `piper` via subprocess (`piper-tts` + `aplay`).
- [x] Async blocking `say()` — waits for playback to finish before returning.
- [x] Executor emits `UiStateChange("speaking")` before TTS and `idle` after.

### 4.3 — Voice loop & feedback prevention
- [x] E2E flow: speak -> VAD detects -> STT transcribes -> Planner routes -> LLM generates -> Executor runs TTS -> Amy speaks.
- [x] TTS feedback loop prevented via `tts_active` atomic flag + mute controller task.
- [x] STT drops samples while `tts_active=true`.

### 4.4 — Intent routing
- [x] Planner reads `"text"` field with fallback to `"intent"` (STT vs other sources).
- [x] Voice input tagged `[Voice]` in LLM prompt.
- [x] `working_memory: []` placeholder sent to satisfy API contract.

### 4.5 — Session fixes (2026-08-15)
- [x] Fixed intent routing field mismatch (`text` vs `intent`)
- [x] Fixed UDS `readexactly()` for large audio payloads
- [x] Fixed empty transcription (`vad_filter=False`)
- [x] Fixed mic device selection (explicit ALSA scoring)
- [x] Fixed TTS feedback loop (async blocking + atomic mute)
- [x] Fixed `MAVIS_PYTHON_PATH` in `.envrc`
- [x] E2E clean loop verified — no self-triggering

### 4.6 — Session fixes (2026-08-17)
- [x] Aggressive `_post_process()` — strips `===`, markdown, bullets, truncates to 1-2 sentences
- [x] TTS prosody — `--length-scale 1.15`, `--sentence-silence 0.25`, auto-detect `lessac` > `ryan` > `amy`
- [x] VAD noise floor fix — `reset()` resets floor to 0.005, capped at 0.015, multiplier 1.5 capped at 0.022
- [x] Pre-emphasis regression fix — removed pre-emphasis filter that attenuated voice energy
- [x] System prompt hardening — no markdown, no lists, no separators, no repetition
- [x] Fan-noise robustness — `min_max_energy` gate rejects low-energy utterances

### 4.7 — Phase 4 wrap-up checklist
- [x] STT engine (faster-whisper) integrated
- [x] cpal audio capture + energy VAD
- [x] UDS protocol fixed (`readexactly`)
- [x] Worker eager spawn
- [x] STT request timeout for model load
- [x] faster-whisper model cached locally
- [x] E2E voice loop wired (speak -> hear response)
- [x] E2E voice loop verified clean (no self-triggering)
- [x] Phi-3 manual chat template (no prompt regurgitation)
- [x] Working memory injection into LLM prompt
- [x] Adaptive VAD noise floor updating per-utterance
- [x] LLM response quality (no `===`/markdown/repetition)
- [x] TTS naturalness (prosody settings)
- [x] Idle unload verified with both models
- [x] Fan noise robust filtering — the VAD follows the room's noise floor, and Silero decides what is speech before Whisper hears it *(2026-09-22, §6.4; DECISIONS §15)*
- [x] Hotkey binding for push-to-talk — `toggle_listen.sh`; the README gives the binding for niri, sway and Hyprland *(2026-10-08)*

---

## Phase 5 — Interaction Polish & Personality Foundation

**Status:** :white_check_mark: COMPLETE (2026-08-26 — 2026-09-02)

**Goal:** Close the gap between "it works" and "it feels good to use every day." Establish the emotional baseline.

### 5.1 — TTS Queue & Interruption
- [x] Non-blocking `say()` — queues text via `mpsc`, returns immediately
- [x] Background task handles sequential playback
- [x] `interrupt()` sends `kill -15` to current audio PID + drains queue
- [x] Intent router publishes `TtsInterrupt` when speech detected during TTS
- [x] Mute controller only unblocks mic on `idle`/`error`

### 5.2 — Session State Recovery
- [x] `WorkingMemory` serialization (`to_json()` / `from_json()` via serde_json)
- [x] Hydration on startup — reads `../memory/working_memory.json`
- [x] Debounced auto-save (1s max rate) after every `process_event()`
- [x] Shutdown save — `memory.save_working().await` before `bus.close()`

### 5.3 — Orb Emotional Expression
- [x] `Celebrating` state — warm gold pulse after successful completion
- [x] Rendered in orb for ~1.2s before returning to idle

### 5.4 — Voice Activity LED
- [x] Real-time VAD energy piped to orb brightness
- [x] Peak-hold + decay (`*= 0.92` per frame)
- [x] Per-state energy scale: Listening=0.50, Idle=0.20, others lower

### 5.5 — Audio Device Selection
- [x] `MAVIS_AUDIO_DEVICE` env var respected for mic (exact CPAL name match)
- [x] Playback device selectable — `MAVIS_AUDIO_OUTPUT` (`--target` for pw-play, `--device` for paplay, `-D` for aplay). *Was `MAVIS_AUDIO_DEVICE` with the wrong flag, which broke playback; fixed 2026-09-27.*

### 5.6 — Bug Fixes (2026-08-29, corrected 2026-09-02)
- [x] **Echo cancellation** — the 2026-08-29 fix added VAD-reset + 250ms cooldown logic to `stt.rs`, but the `tts_active` flag that logic checked was never connected to the executor's real TTS state — two separate `AtomicBool`s, never wired together. Confirmed via live testing (MAVIS's own TTS output getting transcribed back as new "user" speech) and fixed 2026-09-02: `SttHandle::start()` now takes the shared flag as a parameter instead of creating its own.
- [x] **Name extraction** — the offset-bug fix + initial denylist held for the words already known, but live testing kept surfacing new false positives (`Not`, `In`, `Looking`, `Mavis` itself) — mostly downstream of the echo bug above feeding MAVIS's own speech back in as fake "user" input. Denylist extended; most of this class of error should disappear now that echo is actually fixed.
- [x] **BrokenPipeError** — unchanged, still holding.
- [x] **Prompt pollution** — unchanged, echo dedup in `build_chat_messages()` still holding.
- [x] **warm_up crash** — unchanged.
- [x] **VRAM OOM** — `n_gpu_layers` is now actually wired from config instead of silently ignored, but default stays at 20 layers — confirmed 2026-09-02 that `-1` (full offload) causes `Failed to create llama_context` on 6GB VRAM. The `-1` in `config.toml` was aspirational and never verified on this hardware; 20 was the empirically correct value all along.

### 5.7 — Conversation Style, TTS Abstraction, Push-to-Talk (2026-09-02)
- [x] **Conversation style baseline** — added warmth/confidence/anti-leakage rules to `SYSTEM_PROMPT`; fixed `LlamaEngine.chat()` to apply style post-processing to every model path (previously only ran for Phi-3/TinyLlama's manually-templated branch — a native-chat-template model would have skipped it entirely).
- [x] **TTS voice abstraction** — found already implemented (`MAVIS_TTS_ENGINE=piper|kokoro`, one dispatch point in `executor.rs`, rest of the executor untouched by engine choice). Removed a dead, fully disconnected duplicate Piper implementation (`LinuxTts`/`TtsPlayer` trait across `platform/mod.rs`, `linux.rs`, `windows.rs`, `macos.rs`) that nothing ever called.
- [x] **Push-to-talk / active listening** *(2026-10-08)* — a choice, set as `listen_mode` in `config/config.toml` *(an install-time question was built and removed 2026-10-09; it returns when MAVIS is ready for daily use)*. **Always** stays the default. **Push to talk** ignores the microphone until a hotkey press or an orb tap, then hears one utterance (8 s to start) and closes. The utterance is marked as asked-for, so the worker doesn't second-guess it as noise. Proven by the capture test; not yet heard on hardware. See [`DECISIONS.md` §24](DECISIONS.md#24-everything-up-to-phase-8).

### 5.8 — Full E2E Verification (2026-09-02)
Real bugs found through live voice testing on target hardware — not simulated, not assumed fixed from code review alone:
- [x] **TTS echo feedback loop** — root cause and fix under 5.6.
- [x] **Whisper hallucination on near-silence** — the model invents fluent filler ("thank you for watching", "I don't know what I'm talking about") with *high* confidence on quiet ambient noise, bypassing the confidence gate entirely. Added `HALLUCINATION_DENYLIST` in `stt/engine.py`, with whitespace/apostrophe normalization (segment-join artifacts and curly quotes were breaking exact-match comparisons on the first attempt). *(Replaced 2026-09-22 by speech detection — §6.4.)*
- [x] **Low-confidence drops were a black box** — logging now includes the actual dropped text, not just the score, so borderline rejections are debuggable instead of silent.
- [x] **"MAVIS" misheard as baby/movies/moose** — added `initial_prompt="MAVIS"` to bias the Whisper decoder toward the rare proper noun. Measurable improvement (4/6 correct in one test session), not a full fix — expected limitation of a quantized small model on an uncommon name.
- [x] **Worker logs arriving late / out of order** — Python's stdout was block-buffered when piped (no `PYTHONUNBUFFERED`); prints without explicit `flush=True` queued for seconds before appearing, which had been actively misleading live debugging all session.
- [x] **Prompt leakage via paraphrase** — tightening `SYSTEM_PROMPT` rule 13's wording did not work; the model swapped "guiding principles" for "my guidance" and leaked the same way. Real fix: deterministic phrase-match in `planner.rs` (`is_meta_instruction_question`) that bypasses the LLM entirely for meta-questions about its own instructions, returning a fixed deflection instead. Verified 3/3 on real hardware. *(Removed in Phase 8.6, §46: the model handles these questions itself.)*
- [x] **Session recovery** — verified working across two consecutive restarts; correct user name persisted both times.
- [x] **Model path / GPU layer config drift** — `WorkerServer` was constructing `LlamaEngine()` with no arguments, silently ignoring `config.toml` entirely. Now wired through safely — a configured model path is only honored if that file actually exists on disk, otherwise falls back to the existing auto-detection, so a stale/wrong config value can never break a working setup.

**Known gap surfaced, not fixed (carried into Phase 6):** active-window and clipboard capture already run continuously (`ContextEngine: context injected` fires every ~2s) but that data never reaches the LLM — `build_working_memory()` in `planner.rs` doesn't read `active_window` or `last_clipboard`. This is exactly Phase 6's first two planned context sources; the capture half was already built, the wiring wasn't.

**Confirmed absent:** no vision/screen-reading capability exists anywhere in the pipeline. `capture_focused()` is defined in `platform/linux.rs` but never called by anything, and the loaded LLM (Phi-3-mini) is text-only regardless.

---

## Phase 6 — Context Awareness Foundation

**Status:** :white_check_mark: COMPLETE (2026-09-02 — 2026-09-11)

**Goal:** The companion must know what the user is doing, not just what they are saying. Every context source is opt-in via an environment variable, default off.

| Source | Status | Implementation |
|--------|--------|----------------|
| Active window | :white_check_mark: | `niri msg --json windows`; falls back to sway/hyprland/xdotool |
| Open windows + workspace | :white_check_mark: | Same niri call — the full list was already being fetched and discarded |
| Clipboard | :white_check_mark: | `wl-paste` (Wayland) / `xclip` (X11), polled with the context snapshot |
| IDE awareness | :white_check_mark: | Any window whose title names a file is reported as naming it. *(The per-editor title rules were removed 2026-10-13: no app is known by name.)* |
| Terminal awareness | :white_check_mark: | A terminal's title is its running command; leading `VAR=value` assignments are stripped from every title, so the command survives. *(No list of terminal apps since 2026-10-13.)* |
| Workspace / Project | :white_check_mark: | Walks the focused window's `/proc` process tree for a cwd outside `$HOME`, then up to the nearest `.git`; reports repo name, path and branch |
| Calendar / Time context | :warning: | Date/time always injected. Calendar reads Evolution's `~/.local/share/evolution/calendar/system/calendar.ics`. Parser verified against the real file format; the has-events path is **untested** — the local calendar is empty. Recurring events (`RRULE`) are skipped by design. |
| Browser awareness | :x: | **Dropped.** Getting real tab URLs requires a browser extension + native messaging host. Decided against shipping a browser extension as part of MAVIS. The `mavis_core` receiving end (`BrowserUpdate` event, `/tmp/mavis_browser.sock` listener) is built and verified, so a future non-extension source could feed it. |

- [x] **Context snapshot** — `ContextSnapshot` published every 2 s from the platform polling loop
- [x] **Privacy gate** — per-source opt-in, default off: `MAVIS_CONTEXT_ACTIVE_WINDOW`, `MAVIS_CONTEXT_CLIPBOARD`, `MAVIS_CONTEXT_CALENDAR`, `MAVIS_CONTEXT_BROWSER`. Date/time is always on (not private). The fuller 5-tier permission system remains Phase 8.
- [x] **Cross-platform abstraction** — `PlatformProvider` / `WindowTracker` traits; Linux implemented, Windows and macOS stubbed. `current_project()` has a default `None` impl so non-Linux platforms need no `/proc` equivalent.

### 6.1 — Verified live (2026-09-11)
Confirmed by real voice interaction, not analysis alone:
- Open workspaces, the windows on screen, the date, time and current project, and the clipboard were each asked about and answered correctly.

### 6.2 — Voice pipeline tuning (2026-09-11 — 2026-09-12)

A run of VAD problems, each found and fixed against measured data rather than guesswork. Recording the causes because several of them looked like different bugs than they were.

- **Silence was undetectable.** The adaptive threshold was capped at 0.022, but measured silence on this machine's mic sits at median 0.033 / peak 0.058 — the cap sat *below* the noise floor, so every frame read as speech and utterances only ever ended at a hard ceiling. Thresholds re-derived from `arecord` traces of real silence and real speech.
- **Single threshold couldn't serve both jobs.** High enough to avoid false starts meant cutting mid-sentence on quiet syllables. Replaced with hysteresis: start 0.075, end 0.06. The longest sub-0.06 dip measured inside real speech was 450 ms, which sets the floor for the silence wait.
- **Audio-stream startup transient.** Opening the input stream emits a full-scale click (`max_energy=1.000`), shipped as a ~1.3 s "utterance" that Whisper turned into fluent invented sentences. Fixed with a 1.5 s settle window plus a minimum utterance length of 1.5 s.
- **Silence wait 600 ms → 500 ms.** Dead time on every exchange. Floored by the 450 ms measurement above; going lower needs the end threshold raised first.
- **Whisper hallucination on near-silence** — invents filler with *high* confidence, so the confidence gate can't catch it. Denylist of known phrases, with whitespace/apostrophe normalisation. *(Replaced 2026-09-22 by speech detection — §6.4.)*
- **Confidence threshold 0.6 → 0.45.** Correct transcriptions were being silently dropped at 0.590 and 0.499.

**Result (measured 2026-09-12, 12 exchanges):** 9 of 12 replies in 1–2 s from end of speech. 9 of 10 transcriptions accurate, "MAVIS" recognised correctly in five separate utterances. No speech detected during any TTS playback — the suspected echo leak did not reproduce, and the earlier "fire-and-forget playback" diagnosis was wrong: the executor does await `child.wait()`.

**Still open:** cold start is 15–30 s while models load (mitigated by leaving the worker running between `mavis_core` restarts). Calendar's has-events path remains untested.

### 6.3 — Cross-desktop hardening (2026-09-16 — 2026-09-19)

Found by running on GNOME rather than niri. Details in [`DECISIONS.md` §5](DECISIONS.md#5-context-awareness--phase-6).

- [x] **GNOME session crash** — project detection rescanned all of `/proc` once per descendant process: tens of thousands of file reads every 2 s. Now one pass builds a process tree, walked in memory, bounded to depth 3 and 64 nodes, cached per PID for 30 s. *Not yet re-verified on a real GNOME session.*
- [x] **Compositor resolved once** — each poll used to try niri, sway, Hyprland and up to three `xdotool` calls. Now probed once at startup.
- [x] **Environment variables are hints, not proof** — a `NIRI_SOCKET` leaked into a GNOME login made MAVIS commit to niri and fail every poll. Each candidate is now confirmed by actually running it.
- [x] **Failed commands no longer look successful** — `run_cmd` checks exit status and rejects empty output.
- [x] **XDG application directories** — app discovery now searches `XDG_DATA_DIRS` and `XDG_DATA_HOME` in addition to the defaults, which picks up Nix, Snap and several distributions' layouts.
- [x] **Graceful audio failure** — a missing or busy microphone used to abort the whole process. MAVIS now runs without voice input and says so.

### 6.4 — Live-run fixes (2026-09-22)

The first full run after the audit answered "hello MAVIS, can you hear me" in 60 s — to a sentence the user never said. Details in [`DECISIONS.md` §15](DECISIONS.md#15-the-2026-09-22-live-run).

- [x] **VAD follows the room** — floor measured at startup, uncapped, kept between utterances, re-measured mid-utterance when the room gets louder; smoothed energy; tail trimmed; ceiling 45 s → 30 s. *Proven in simulation; not yet measured on hardware.*
- [x] **Speech detection instead of phrase lists** — Silero VAD (bundled with faster-whisper) marks speech; Whisper hears only that, and isn't called at all without it. Hallucination denylists removed. Unpunctuated transcripts no longer vanish
- [x] **LLM stops as soon as the reply is complete**; system prompt pre-evaluated during speech; per-reply timing logged
- [x] Names recognised by sentence shape (explicit introduction, ends the clause, not a question, capitalised) — no word lists; names from older builds discarded once
- [x] Orb excluded from active window and project detection
- [x] Prompt history de-duplicated; clipboard only when asked
- [x] Speech fallback chain fixed; integer-format microphones accepted

### 6.5 — Barge-in and action reliability (2026-09-26)

Second live run: 2 s median replies, no hallucinations — and five "Opening …" replies with nothing opened. Details in [`DECISIONS.md` §16](DECISIONS.md#16-the-2026-09-26-live-run-and-barge-in).

- [x] **Barge-in** — MAVIS listens while it speaks and stops the moment you talk over it; "stop" is understood as a request for silence, not a question. `MAVIS_BARGE_IN=0` restores mute-while-speaking. *Simulated only; not yet tried on real speakers.*
- [x] **Actions actually run** — the verb may appear anywhere provided an installed app or URL follows; punctuation after the wake word no longer blocks matching; an app matches when its name appears in what was said
- [x] **The model no longer narrates actions it cannot perform** (rule 14), no longer repeats its last reply (rule 15), and the known facts sit last in the prompt where they outrank the conversation (rule 16)
- [x] **"Sorry, I didn't catch that"** below 0.55 transcription confidence, instead of answering a misheard question
- [x] Minimum utterance 1.5 s → 0.8 s; the "it was the room" drop relaxed; the noise floor only learns from frames that clearly aren't speech
- [x] Questions are no longer stored as memories
- [x] The Python worker's logging was never switched on — `setup_logging()` is now called
- [x] Kokoro starts from its local cache instead of asking Hugging Face

### 6.6 — Barge-in, second pass (2026-09-27)

Barge-in met real speakers and MAVIS cut itself off on every reply. Details in [`DECISIONS.md` §17](DECISIONS.md#17-the-2026-09-27-run--mavis-interrupting-itself).

- [x] **The echo measuring window counts audible playback**, not wall-clock time — synthesis silence was eating all of it
- [x] **The echo estimate is a decaying peak**, not an average, so MAVIS's own syllable peaks don't read as speech
- [x] **An interruption pauses playback instead of killing it** — the transcript decides whether to carry on or answer, so a false trigger costs a pause rather than the whole reply
- [x] **Capture prefers PipeWire** (and an echo-cancelled source above all) over the raw sound card
- [x] **`MAVIS_AUDIO_OUTPUT`** for playback, with the right flag per player; `MAVIS_AUDIO_DEVICE` no longer breaks audio
- [x] The worker warns when the microphone is clipping

### 6.7 — First hardware run since Phase 6 (2026-10-13)

The first real session in two weeks. Launching apps worked on hardware for the first time; four things the user asked for did not. Details in [`DECISIONS.md` §26](DECISIONS.md#26-the-2026-10-13-run).

- [x] **"Stop" pauses what's playing** when MAVIS isn't speaking, "Pause" and "resume" no longer toggle. *(Since Phase 8.6 the model chooses `stop`; the low-confidence exception was removed, §46.)*
- [x] **"Play X in <app>" / "Use <app>"** opens the video in that app, if it registers for web links. "Open <app> and play X" does the same. Searches stop at the end of the sentence.
- [x] **Two requests in one sentence** both run ("volume up and next song").
- [x] **A misheard name** ("MOVIS, what changed?") still counts as addressing MAVIS.
- [x] **Open apps and workspaces are counted before the model sees them**, and workspaces are the numbers niri shows, not its internal ids.
- [x] **No app names in the code.** Editors, terminals and browsers are no longer special-cased; Whisper's vocabulary is the apps installed on this machine.
- [x] The worker's full prompt is logged only with `MAVIS_PROMPT_DEBUG=1`.
- [x] **"Close <app>"** *(2026-10-14)* — closes that app's open windows, as their close button would, after "Shall I close …? It may lose anything unsaved." Matched against the windows actually open. niri only for now: it's the one desktop that reports window ids.

### 6.8 — Second hardware run (2026-10-15)

Details in [`DECISIONS.md` §27](DECISIONS.md#27-the-2026-10-15-run).

- [x] **What's on screen goes last in the prompt, apart from the conversation** — the model answered "Brave" with kitty focused, copying its own previous answer.
- [x] **Media pauses while MAVIS speaks** and resumes afterwards, unless the user paused, resumed or opened something in between.
- [x] **Interrupting MAVIS takes 360 ms of speech over it**, not 150 — 9 of 10 interruptions were noise, each a stutter.

### 6.9 — Third hardware run (2026-11-01)

Details in [`DECISIONS.md` §28](DECISIONS.md#28-the-2026-11-01-run).

- [x] **"Stop" over MAVIS's voice works again.** Since 6.8 a short word couldn't be heard at all while MAVIS spoke. MAVIS now listens at once, and only pauses its voice once the user has spoken for 360 ms; a lone "stop" is transcribed and acted on.
- [x] **The Whisper model is a setting** (`[stt]` in `config/config.toml`), so `small.en` can replace the multilingual `small`.
- [x] **The orb no longer hides the focused window.** It took keyboard focus when it opened, and MAVIS then knew of no focused window until the user clicked something. The window MAVIS was started from is read first, and on niri focus is handed back to it.

### 6.10 — A natural voice (2026-11-05)

- [x] **Kokoro is the default voice.** Piper was, because the executor used Kokoro only when `MAVIS_TTS_ENGINE=kokoro` was set — so the worker loaded and warmed Kokoro on every start and no reply ever used it. Piper stays as the fallback. See [`DECISIONS.md` §29](DECISIONS.md#29-a-natural-voice).
- [x] **Replies are spoken a sentence at a time**, the next one synthesized while the current one plays, so a long reply starts as soon as its first sentence is ready.

### 6.11 — What MAVIS says about itself (2026-11-05)

Details in [`DECISIONS.md` §30](DECISIONS.md#30-what-mavis-says-about-itself).

- [x] **MAVIS knows it speaks.** It had said "I don't have a voice", in its voice.
- [x] **No third-person narration** ("Mavis responds…: '…'"); only what MAVIS says is spoken.
- [x] **Fuller replies**: up to three sentences when asked to explain, instead of a hard two-sentence, 180-character cut.
- [x] **"Say that again" repeats the last reply.**
- [ ] Hear these on hardware

### 6.12 — Full run (2026-11-05)

Details in [`DECISIONS.md` §31](DECISIONS.md#31-the-2026-11-05-full-run).

- [x] **"Play X" plays the top YouTube video** instead of opening a search; the search page is the fallback.
- [x] **Misheard app names still open**, and an app that isn't installed is said to be missing instead of the model claiming it can't open apps.
- [x] **"Close X" can be confirmed by saying it again**, and commands and answers are no longer dropped as unclear.
- [x] **`small.en` by default**, and a Whisper prompt that writes "Mavis" instead of "Movies".
- [x] Window counts, terminal titles, "recent changes" and closing offers of help fixed
- [ ] Hear these on hardware; confirm a video starts in the browser

### 6.13 — Run fixes (2026-11-06)

Details in [`DECISIONS.md` §32](DECISIONS.md#32-the-2026-11-06-run).

- [x] **The worker reads `config/config.toml`**, so `small.en` actually loads; the model settings in it now match what runs.
- [x] **"Open <app>" opens that app**, not another entry using the same binary.
- [x] **Answers to "Shall I…?" confirm**, and **"close it"** closes the focused window. *(Answers are understood by the model since Phase 8.6.)*
- [x] **No made-up system state**; misheard "what changed recently" still reaches the Sentinel.
- [ ] Hear these on hardware; confirm whether some windows were missing from the count

### 6.14 — Playing in any app (2026-11-07)

Details in [`DECISIONS.md` §33](DECISIONS.md#33-playing-in-any-app).

- [x] **No site or browser in code**: `[media]` and `[web]` in `config/config.toml`.
- [x] **"Play X in <app>" for any app**, by what it registers for: browser → site, media player → your files, own link type → its search.
- [x] **The app last played in is used next time.**
- [ ] Try with a second browser and a music app on hardware
- [x] Decide: the model choosing actions instead of phrase matching — decided and built in Phase 8.6 (§44)

### 6.15 — No more dragging (2026-11-09)

Details in [`DECISIONS.md` §34](DECISIONS.md#34-focus-is-handed-back-once).

- [x] **Switching window or workspace no longer pulls you back**: focus is handed back from the orb only as it opens.
- [ ] Check on hardware

### 6.16 — Speech model download (2026-11-10)

Details in [`DECISIONS.md` §35](DECISIONS.md#35-the-speech-model-that-couldnt-download).

- [x] **A new speech model can download**: offline only once every model is on disk.
- [x] **Falls back to a speech model already downloaded**, and says so.
- [x] **No five-fold retry** of a request the worker has already refused.
- [x] **Says why it can't hear** instead of going quiet; the red orb fades after four seconds.
- [ ] Confirm `small.en` downloads and loads on hardware

### 6.17 — Review before testing (2026-11-13)

Details in [`DECISIONS.md` §36](DECISIONS.md#36-review-of-611616).

- [x] Thirteen bugs found by reading the code since 6.11, each fixed and most with a test: a misheard name shared by two entries, "yes, close it", "close it" from the orb, offline start, Whisper's prompt, web search, the site after a music app, link types, "Mavis?", download errors, narration, the not-found message, the folder search
- [ ] Hardware run

### 6.18 — The name, once (2026-11-14)

Details in [`DECISIONS.md` §37](DECISIONS.md#37-the-name-once).

- [x] **The user's name only in the first reply of a run**; later replies drop it where it only addresses them.
- [ ] Hear it on hardware

### 6.19 — The orb floats (2026-11-14)

Details in [`DECISIONS.md` §38](DECISIONS.md#38-the-orb-floats).

- [x] **Overlay on Wayland** (layer shell): above every window, on every workspace, never takes focus; a window on GNOME and X11
- [x] **Drag anywhere**, moved on release and kept on screen; `MAVIS_ORB_POS` works on Wayland
- [ ] See it on niri

### 6.20 — Honest, safe, and more capable (2026-11-17)

Details in [`DECISIONS.md` §39](DECISIONS.md#39-the-2026-11-17-run).

- [x] **No made-up actions**: failed actions are said, with the reason. *(The reply filter for claimed actions was removed in Phase 8.6, §46: DO/SAY means a claim can't happen without the action.)*
- [x] **MAVIS never closes its own terminal**
- [x] **Ignores its own voice** during playback
- [x] **Apps by kind**: "the terminal", "the browser", asking which when several fit
- [x] **New controls**: workspaces, full screen, volume and brightness levels, Bluetooth, Wi-Fi, lock, log out, suspend, shut down, restart (the session ones ask first)
- [x] Window names from installed apps; "[mavis]" labels, lead-in change questions and follow-ups cleaned up
- [x] **The orb follows the cursor while dragging**
- [x] Let the model choose actions for requests no rule recognises (6.21; now every request, Phase 8.6)
- [ ] Try on hardware

### 6.21 — Requests in your own words (2026-11-17)

*Superseded by Phase 8.6 (§44): the model understands every request directly.*

Details in [`DECISIONS.md` §40](DECISIONS.md#40-the-model-picks-the-command-the-rules-carry-it-out).

- [x] **The model picks a command** from the rules' own list when no rule matched, or none
- [x] **The pick goes back through the rules**, with the same confirmations; none means the usual reply
- [x] Questions skip it; the command list is built from the rules, so the two can't drift
- [x] Tried on hardware (§41): picks take 0.75–1.15 s; questions were mistaken for requests, fixed in 6.22

### 6.22 — Questions stay questions (2026-11-18)

*The question filters below were replaced in Phase 8.6 (§44, §46); the model tells questions from requests.*

Details in [`DECISIONS.md` §41](DECISIONS.md#41-the-2026-11-18-run).

- [x] **Questions skip the command choice**: after "No," or "OK", ending in "?", "can you explain", "I want to know"
- [x] **The model's pick must come from what was said**: no apps or sites nobody mentioned
- [x] "Close whatever you just opened"; "close the active window" without the model
- [x] Windows named after their app, even when the window's id differs from the launcher's
- [x] Change questions with one misheard word reach the Sentinel
- [x] A setuid program that changes folder is one move, not a critical new one (§42)
- [ ] Try on hardware

### 6.23 — The 2026-11-22 run (2026-11-22)

*The command-choice and "workspace too" / "all the way up" items were replaced in Phase 8.6 (§44).*

Details in [`DECISIONS.md` §43](DECISIONS.md#43-the-2026-11-22-run).

- [x] Its own voice, read out as a piece of a file name ("PRSA"), is recognised as echo
- [x] Sentinel: no unspeakable file names, "Another update today…", and a "what changed" answer counts everything it summed up as told
- [x] Corrections ("…, not the active window") skip the command choice; the choice sees the last exchange
- [x] The model is told the real speaker volume and screen brightness, and never invents levels or shortcuts
- [x] "Workspace too" is workspace 2; "all the way up" is 100
- [x] Brightness without `brightnessctl`, through systemd-logind
- [x] The mic warning suggests a step down, not a fixed level
- [ ] Try on hardware

---

## Phase 6.5 — Action Execution

**Status:** :white_check_mark: Built 2026-09-12; launching apps confirmed on hardware 2026-10-13 (6.7)

MAVIS could previously only ever `say` things — `executor.rs` had `shell`, `app`, `notify` and `system` implemented, but `planner.rs` emitted nothing but `say`, so none of it was reachable.

- [x] **Deterministic action intents** — phrase matching in `planner.rs`, not LLM tool-calling. *(Replaced in Phase 8.6, §44: the model chooses from a catalogue of actions, held to form by a grammar, and code checks each choice before it runs.)*
- [x] **App launching** — "open firefox", "launch terminal"
- [x] **YouTube** — "play lofi hip hop" → opens a search
- [x] **Web search** — "google rust async traits"
- [x] **URLs** — "open https://github.com"
- [x] Actions emit `say` + action together, so there's spoken confirmation.
- [x] Unrecognised commands fall through to the LLM rather than being guessed at.

- [x] **Launch path run end to end** *(2026-10-09, sandbox)* — volume, media, brightness, search, YouTube and two installed apps, driven by typed commands with stand-in programs. Launchers with quoted paths or arguments are now split correctly, and a request made while MAVIS is asking "Shall I?" is carried out rather than lost. Not yet seen on a real desktop. See [`DECISIONS.md` §25](DECISIONS.md#25-testing-phases-68-without-hardware).

**`shell` is deliberately unreachable from voice.** An unrestricted `sh -c` driven by speech transcription is the hole the initial audit flagged; it stays closed until Phase 8 builds real permissions.

### 6.5.1 — Installed application discovery

First implementation used a hardcoded alias list, which broke the moment the user changed browser or editor. Replaced with discovery from what's actually installed, so swapping to any other app works with no code change.

| Platform | Source | Launch | Status |
|----------|--------|--------|--------|
| Linux | `.desktop` files across 5 dirs incl. Flatpak exports | binary from `Exec=`, field codes stripped | :white_check_mark: 104 apps discovered |
| Windows | Start Menu `.lnk`, recursed 3 levels | `cmd /c start "" "<path>"` | :warning: **Untested** |
| macOS | `.app` bundles in 4 standard dirs | `open -a "<Name>"` | :warning: **Untested** |

Discovery lives behind `PlatformProvider::installed_apps()` with an empty default, so an unimplemented platform degrades to "no apps found" rather than failing. Matching is deliberately loose for speech — a spoken word finds an app whose name begins with or contains it — and returns nothing rather than guessing at a binary that doesn't exist. Scanned once at startup; new installs need a restart.

**System control** (added 2026-09-15): volume, mute, media keys and brightness reach the existing DBus operations directly. *(Chosen by the model since Phase 8.6.)*

**Not yet reachable from voice:** `notify`.

---

## Phase 7 — Memory & Learning Layer

**Status:** :white_check_mark: 7.1 and 7.2 complete (2026-09-13 — 2026-09-15). 7.3 deferred — see below.

**Goal:** Move from session-scoped working memory to persistent, growing memory.

Two deliberate departures from the original plan, both made to avoid dependencies this project doesn't need. Each is noted against the item it replaces.

### Memory tiers as built

| Tier | Contents | Lifetime | Store |
|------|----------|----------|-------|
| Working | current session state, context snapshot | in-RAM, JSON snapshot on shutdown | `working_memory.json` |
| Recall | user utterances and MAVIS replies, importance-scored | decays by importance | `recall.db` (FTS5) |
| Long-term | one compressed summary per day | permanent | `long_term.db` (FTS5) |
| Entities | projects, apps, files and their co-occurrence | permanent | `entities.db` |

### 7.1 Intelligent Memory Pipeline

- [x] **Importance scoring** — no extra model call per exchange. Facts are kept when the model chooses `remember` (importance 9, and only if the fact restates what was said); ordinary statements score 5; questions and very short utterances aren't stored. *(The phrase markers for facts were removed in Phase 8.6, §46.)*
- [x] **Episodic store** — `recall.db`, FTS5 virtual table over `(text, role, timestamp, importance)`.
- [x] **Forgetting / decay** — retention scales with importance rather than a flat 30 days: MAVIS's own replies 7 days, questions 30, statements 90, stated facts never. Purged once at startup.
- [x] **Automatic summaries** — consolidation task runs hourly, looking back 7 days for any completed day not yet summarized. Hourly rather than a fixed-time cron because MAVIS isn't guaranteed to be running at 3am; "check whether yesterday still needs doing" is more robust. Skips already-summarized days without an LLM call, and skips days with fewer than 3 meaningful memories.
- [x] **Long-term consolidation** — summaries stored in `long_term.db` with FTS5 search. No vector embeddings (see 7.2).
- [x] **Episodic replay** — "what was I doing yesterday?" is answered from memory of that period. The model works out how far back to look (`look_back <days>`); the code fetches that window. *(The fixed time-phrase list was removed in Phase 8.6, §46.)*
- [x] **Context compression** *(2026-10-08)* — the prompt carries the last 12 events in full. Anything older, including what has already left the 50-event ring, becomes one line: "Earlier in this conversation the user said: …" — the last five things the user said, 80 bytes each. User words only, for the reason recall keeps only them (§7.4). No LLM summary: that would cost a model call per turn.

### 7.2 Advanced Memory

- [x] **Semantic recall** — implemented with SQLite **FTS5**, not embeddings. The plan called for `sentence-transformers` (~80 MB model) plus FAISS: three new dependencies for a project whose rule is to add only what's necessary. Full-text search covers a large share of the same ground — verified that "audio" finds a memory about "VAD thresholds", and "text to speech engine" finds "piper over kokoro". **If recall proves too literal in daily use, embeddings become an evidence-backed decision rather than an assumed one.**
- [ ] **Vector embeddings** — deliberately not added, see above. *Reconsidered 2026-10-08 and kept out: only stated facts (importance 8+) reach the prompt, a small set FTS5 handles; embeddings would add a model download and a worker round trip to every utterance. Revisit when a real miss is seen ([§24](DECISIONS.md#24-everything-up-to-phase-8)).*
- [ ] **FAISS / hnswlib** — deliberately not added, see above.
- [x] **Memory graph** — entities derived from data the context layer already resolves with certainty: git repository names, compositor app IDs, filenames from IDE window titles. No spaCy: it's a heavy dependency plus a language model, and it's poor at exactly these entities — it won't tag `mavis_core` as a project or `stt.rs` as a file. These are observations rather than predictions, so nothing can be misidentified. **Known gap:** people mentioned in conversation ("meeting with Sarah") are not captured; that is what spaCy would have added.
- [x] **Relationship graph** — co-occurrence weights between entities, so "what's usually open alongside the MAVIS project" has an answer. Recorded only when the app/project/file combination *changes* — context updates arrive every 2 s, and recording each one would measure idle time rather than work.

### 7.3 Learning Engine

**Moved to Phase 8.6, step 3 (learning from use).** Originally deferred *(reaffirmed 2026-10-08)*: Routine detection needs months of accumulated behaviour to find anything real; MAVIS has days. Building a pattern detector with no patterns to detect would produce confident nonsense, and the entity graph from 7.2 is precisely the data that will eventually feed it. Revisit once `entities.db` has meaningful history.

- [ ] Routine detection
- [ ] Preferred apps per context
- [ ] Learn coding schedule
- [ ] Learn workflows
- [ ] Learn frequently used commands
- [ ] Adapt suggestions over time

### 7.4 — Fixes after first use (2026-09-16 — 2026-09-19)

- [x] **MAVIS stopped quoting itself back as memory.** 10 of 17 recalled entries were MAVIS's own earlier replies — one carrying a "clayboard" mistranscription forward as fact. A guess got recorded, recalled, and treated as established. Recall now returns only what the *user* said; MAVIS's replies are still stored for replay and consolidation.
- [x] **Polling no longer evicts the conversation.** `ContextUpdate` every 2 s filled the 50-slot working-memory ring in about two minutes. Only conversational events enter it now; context lives in dedicated fields.
- [x] **`PermanentStore` and `SessionStore` removed** — Phase 1 stubs that nothing ever read.

### 7.5 — Not yet verified

Everything above compiles and its logic is unit-verified, but none of it has run in a live session. Consolidation in particular cannot be confirmed until MAVIS has been left running across a day boundary. Recall and replay only become meaningful after several sessions of accumulated history.

### 7.6 — Memory hygiene (2026-09-30)

A fresh run answered "can you hear me" with a clipboard from four days earlier. Details in [`DECISIONS.md` §18](DECISIONS.md#18-memory-that-follows-you-into-the-next-session).

- [x] **A restart starts a new conversation** — the event ring no longer survives it; the name, recall, summaries and entities do
- [x] **Stored questions are purged at startup** by today's rules — they stopped being memories on 09-26, but old rows kept coming back
- [x] **Recall drops what is essentially the question being asked** (70% word overlap either way)
- [x] The clipping warning reports the measured peak
- [x] **Clipboard flattened to one line** before it goes in a prompt, with its length stated when long
- [x] **Recall injects stated facts only** (importance 8+); chatter stays in the store for replay
- [x] **Over-range audio scaled back** to 0.95 before the speech detector and Whisper

---

## Phase 8 — Safety & Permission System

**Status:** :white_check_mark: Built (2026-09-17 — 2026-10-07), except per-skill permissions, which wait on Phase 9. **The confirmation flow is reachable by voice since 2026-10-14**, through "close <app>" (risk 3); until then the planner only produced actions scoring 0–2, so it was proven on the event bus but never heard. Rationale in [`DECISIONS.md` §8](DECISIONS.md#8-safety--permissions--phase-8) and [§23](DECISIONS.md#23-finishing-phase-8).

**Goal:** Local-first does not mean reckless. Every capability is gated.

Nothing reaches the executor unreviewed. The planner publishes `PlanReady`; the permission gate scores it, records it, and republishes it as `PlanApproved` — the only event the executor acts on.

```
Planner --PlanReady--> PermissionGate --PlanApproved--> Executor
                              |
                              +--> audit.db (append-only)
```

### 8.1 Permission Tiers

**Built as risk bands rather than five named tiers.** A 0–10 score composes — one dangerous step makes the whole plan dangerous, and `assess_plan` takes the worst — while named tiers remain the right model for per-skill grants, which wait on Phase 9.

| Planned tier | As built |
|---|---|
| `Read`, `Notify` | risk 0–2: `say`, `notify`, `system` (volume/media/brightness), `app`, `undo` — run silently |
| `Ask`, `Execute` | risk 3–7: "Shall I run …?" — waits up to 20 s for a clear yes |
| `Administrator` | risk 8+: asked twice, the second time saying it can't be undone *(Phase 8.6, §46; was "yes, administrator")* |
| — | irreversible patterns: refused at any score |

- [ ] **Per-plugin / per-skill permissions** — blocked on Phase 9. *(The original list had this item twice.)*
- [x] **Dry-run mode** — the question reads out what would run: "Shall I run rm notes.txt? It modifies or deletes data." *(2026-10-07)*
- [x] **Audit log** — `audit.db`, append-only by construction: `AuditLog` exposes no update or delete method. Columns are `(timestamp, action_type, detail, risk_score, outcome, reason)` rather than the planned `(skill, args, user_confirmed)`, since skills don't exist yet.
- [x] **Confirmation prompts** — voice, plus an orange orb state held from the question until it is answered or lapses *(2026-10-07)*. **Tapping the orb answers yes** for risk 3–7, never for risk 8+, which needs a spoken answer *(2026-10-08)*. A tap is a press under 400 ms that moves less than 4 px; anything else is a drag.

### 8.2 Safety Layer

- [x] **Command validation** — static deny-list in `safety/risk.rs`: `mkfs`, `dd if=/dev/zero`, writes to `/dev/sd*`, fork bomb, `shutdown`, `reboot`, `userdel`, `visudo` and more. Whitespace is collapsed first so spacing can't evade it.
- [x] **Root deletion** — `is_root_delete` inspects what *follows* the slash, so `rm -rf /` is refused while `rm -rf /home/user/project` is merely confirmable.
- [x] **Pipe-to-shell** — `is_pipe_to_shell` checks for a download command *and* a shell pipe anywhere, because the URL sits between them.
- [x] **Risk scoring** — static: elevated + destructive 9, elevated 8, destructive 6, any other shell command 4, unrecognised action type 5. An `app` that is really a command (`sudo …`, `sh -c …`) is scored as that command *(2026-10-07)*. Network calls are not scored separately.
- [ ] **LLM validation** — **replaced by static scoring, deliberately.** A missed judgement here runs a destructive command. An LLM pass may later *raise* a score as a second opinion; it must never lower one. *Reconsidered 2026-10-08 and kept out: the same command must always get the same answer, and nothing voice-reachable needs a second opinion until Phase 9's skills.*
- [x] **Rollback** — before `rm`, `rmdir`, `mv`, `shred` or `truncate` runs, the files it names are copied to `.mavis-backup/` in the memory directory. "Undo that" restores them within five minutes; copies are deleted after that. No copy when the command uses globs, variables, pipes or `sudo`, or would exceed 200 MB — the command runs but can't be undone. *(2026-10-07)*
- [x] **Confirmation for destructive actions** — from risk 3.

### 8.3 Confirmation Flow

As built. The plan contradicted itself (§8.2 said confirm from 5, this section said 3); the stricter was used. The answer window is 20 s rather than 5, which was too short to hear a question and answer it by voice.

```
PlanReady -> assess_plan() -> audit.db
    -> deny pattern:   "I won't do that. It's not reversible."      (never runs)
    -> score 0-2:      run silently
    -> score 3-7:      "Shall I <what>? It <reason>."  -> a yes (understood by the model) or an orb tap within 20 s
    -> score 8+:       asked, then asked again: "This can't be undone. Are you sure …?" -> two spoken yeses
    -> anything else:  "Cancelled."                   -> audit: declined / expired / superseded
```

- **The model understands the answer.** While a question is open, the model is told what was asked and can choose `confirm` or `cancel`, however the user puts it. Anything else leaves the question open until it lapses. *(Phase 8.6, §46; the yes/no word grammar was removed.)*
- **A question lapses on time.** After 20 s it is recorded `expired`, whether or not the user speaks again.
- **One question at a time.** A newer one closes the older as `superseded`; "yes" means the one asked last.
- **Every plan shape is scored the same** — an array, one action, or `{"actions": [...]}`. The gate and the executor read a plan with the same function.

**`shell` remains unreachable from voice.** The gate scores shell commands, but the planner still never produces one.

### 8.4 Known gaps

All three recorded here on 2026-09-18 were closed on 2026-10-07, with a fourth found on the way — see [`DECISIONS.md` §23](DECISIONS.md#23-finishing-phase-8).

- ~~`app` actions bypass shell scoring~~ — scored as the command they amount to.
- ~~"ok" counts as consent~~ — the whole utterance had to be agreement; since Phase 8.6 the model judges it.
- ~~A second held plan silently replaces the first~~ — audited as `superseded`.
- ~~A plan wrapped as `{"actions": [...]}` turned a refusal into a question~~ — found and fixed 2026-10-07.

**What remains:** hearing it on hardware. "Close <app>" (2026-10-14) is the first voice command that asks; Phase 9's skills will add more.

---

## Stability audit — 2026-09-20

A full line-by-line audit, run against a clone of the repository. Every claimed bug was reproduced by compiling and running it. Details and evidence in [`DECISIONS.md` §9](DECISIONS.md#9-crash-safety--the-2026-09-20-audit).

- [x] **Three UTF-8 panics** — `strip_address`, clipboard logging and audit summaries sliced strings by byte index, which panics inside a multi-byte character. Whisper emits them routinely. Replaced with `util::truncate_bytes` and `str::get`. **Most likely cause of the intermittent "MAVIS stops answering" reports** — reproduced by a test that fails on the old planner and passes on the new one.
- [x] **Subsystem supervision** — a panic in a spawned task used to end that task silently while everything else kept running. The context engine, planner, permission gate, executor, context poller and Sentinel now run under `util::supervise`: logged loudly, rebuilt, restarted up to 5 times.
- [x] **Event bus poison cascade** — one panic while holding the bus lock made every later `publish` panic. Now recovers.
- [x] **Audio-thread deadlock** — a double `lock()` in the VAD, deadlocking under edition-2021 temporary lifetimes after any poisoning.
- [x] **No new dependencies, no new language features** — nothing added to `Cargo.toml`.

**Still open from the audit** — see [`DECISIONS.md` §14](DECISIONS.md#14-open-issues): a 300 s first STT timeout with serial processing, the worker's idle-unload race, and several pieces of dead code. *(The worker socket at `0666` was closed on 2026-10-07.)*

---

## Phase 8.5 — System Sentinel

**Status:** :construction: All five steps built (2026-09-20 — 2026-10-06). Steps 2b, 3 and 4 are tested — step 4 against the real tools — but **not yet run on hardware**. Step 5 (Windows, macOS) is **unverified**: written from documentation, never run. Rationale in [`DECISIONS.md` §10](DECISIONS.md#10-system-sentinel--phase-85).

**Goal:** Phase 8 audits what MAVIS does. The Sentinel audits what happened *to the machine* — so a package you never asked for doesn't sit unnoticed for days.

**Why it exists:** a system update can pull in a large package nobody asked for as a dependency, and it can go unnoticed for days. That one may be harmless; the delay wouldn't be, if it weren't.

**Rules it holds to:**
- **Severity is static.** Same reasoning as the permission gate: an LLM that misjudges severity either cries wolf until you stop listening, or stays quiet about the one change that mattered.
- **Facts, never malware verdicts.** MAVIS reports what changed. Where a real scanner exists — Defender, ClamAV, XProtect — its findings are reported as *that tool's* findings. MAVIS never implies an all-clear it can't back up.
- **Opt-in.** `MAVIS_SENTINEL=1`, like every context source.

| Severity | What qualifies | How you hear about it |
|---|---|---|
| **Critical** | new setuid/setgid binary, sudoers change, new UID-0 user, new SSH key, integrity mismatch, real scanner detection *(step 3–5)* | desktop notification immediately, spoken next time, in history |
| **Notable** | package installed that you didn't ask for; package removed; package downgraded | spoken the next time you talk to MAVIS *(step 2b)* |
| **Routine** | version upgrades; packages you asked for | recorded only — answered if you ask |

### 8.5.1 — Package change detection (step 1) :white_check_mark:

Reads the package manager's own transaction log rather than diffing package lists. The log carries real timestamps ("your update on Thursday") and separates a new package from a version bump, which a name diff can't. World-readable; no root needed.

| Manager | Log | Status |
|---|---|---|
| pacman | `/var/log/pacman.log` | :white_check_mark: validated against real lines, including the `+0545` offset and the pre-2019 timestamp format |
| dpkg | `/var/log/dpkg.log` | :white_check_mark: validated against a real 675 KB log (1,172 transactions) and a live install |
| dnf / rpm | `/var/log/dnf.rpm.log` | :warning: **untested on real hardware** — written from the documented format |

- [x] A new package absent from the explicit set (`pacman -Qeq`, `apt-mark showmanual`, `dnf repoquery --userinstalled`) is a dependency you didn't ask for — **Notable**. If that query fails, everything is treated as requested, so a broken query is silent rather than a flood.
- [x] Downgrades detected with a dpkg-style version comparison. A first version dropped separators and reported an ordinary `gcc-14-base` upgrade as a downgrade — on every Ubuntu machine. Fixed, and kept as a regression test: 0 false downgrades across 1,172 real entries.

### 8.5.2 — Scan loop, store and summaries (step 2a) :white_check_mark:

- [x] **`sentinel.db`** — a watermark per source plus the change history, keyed by fingerprint so re-reading a log can't announce anything twice. Append-only; only the `announced` flag is updated.
- [x] **Watermark is a timestamp, not a byte offset** — survives log rotation. Filtered *at or after* the watermark: a scan landing mid-transaction previously lost later lines stamped with the same second.
- [x] **First run is silent** — history is imported and marked already-announced. A real Arch log holds thousands of transactions; announcing them would be worse than saying nothing.
- [x] **One sentence per update** — changes are regrouped into the transaction that produced them (300 s window): *"Your system update on Friday pulled in 3 packages you didn't ask for: …"*. With ~90% of Arch packages being dependencies, a sentence per package would be unbearable.
- [x] **Cheap** — one `stat()` per minute; the log is re-read only when it changes.
- [x] **Critical changes go through the permission gate**, so they're audited and the Sentinel can't become a back door to the executor.
- [x] **`SystemChange` event** — kept out of working memory so a large update can't evict the conversation.

**Verified live (2026-09-20):** installed two packages that pulled in a third as a dependency. The Sentinel spoke only about the one that arrived uninvited, then stayed silent on the next scan.

### 8.5.3 — Remaining steps

- [x] **2b — Speaking.** Planner leads with pending Notable changes on your next utterance; "what changed recently?" and "what did that update do?" answered from the store; `mark_announced` so you're told once. *(2026-10-01 — built and tested, not yet heard on hardware; [`DECISIONS.md` §19](DECISIONS.md#19-the-sentinel-speaks--phase-85-step-2b))*
  - Leads with at most two updates; older ones counted, all marked told.
  - The answer comes from the record, never from the model. Since Phase 8.6 the model recognises the question (`system_changes`, `update_details`).
  - Found and fixed on the way: the first-run import was briefly visible as news to a concurrent reader (proven, 2,990 rows seen against the old code).
- [x] **3 — Privilege surfaces.** New setuid/setgid binaries, newly enabled systemd services and timers, sudoers changes, new users and groups, new `authorized_keys` entries. *(2026-10-01 — built and tested, not yet run on hardware; [`DECISIONS.md` §20](DECISIONS.md#20-privilege-surfaces--phase-85-step-3))*
  - Snapshot per surface, diffed each minute; setuid walk hourly and after package changes.
  - Gains are Critical (UID 0, privileged groups, SSH keys, sudoers, setuid) or Notable (login accounts, other groups, enabled units); losses are Routine.
  - An unreadable surface is skipped, never read as empty — proven to prevent a Critical flood.
- [x] **4 — Integrity and advisories.** `pacman -Qkk` / `rpm -Va` / `debsums`; `arch-audit` and the Debian security tracker. *(2026-10-06 — built, run end to end against the real tools in a sandbox, not yet on hardware; [`DECISIONS.md` §21](DECISIONS.md#21-integrity-and-advisories--phase-85-step-4))*
  - Daily, lowest priority, remembered across restarts. `MAVIS_SENTINEL_CHECK_NOW=1` forces a run at startup.
  - Changed content in a packaged file is Critical; missing files and changed permissions Notable; config files and timestamps Routine.
  - Advisories need `MAVIS_SENTINEL_ADVISORIES=1` (they download a list) and the scanner installed. High and critical are spoken, as the scanner's finding.
  - `dpkg --verify` stands in for `debsums`; `debsecan` is the tracker's client, Debian only.
  - A failed tool gives no result rather than an empty one, and a verify that overlaps a package transaction is discarded — both proven.
- [x] **5 — Windows and macOS.** Appx, winget, the uninstall registry, scheduled tasks and startup items — Windows is notorious for restoring removed apps on update — plus Defender's verdicts; Homebrew, launchd, `pkgutil` and XProtect on macOS. *(2026-10-06 — :warning: **unverified on real hardware**; [`DECISIONS.md` §22](DECISIONS.md#22-windows-and-macos--phase-85-step-5))*
  - An app that reappears after being removed is reported as back, not as new.
  - **Not built:** winget (it shows the same two lists already read) and XProtect (no way to confirm its log format from here; a scanner verdict isn't something to guess at).

---

## Phase 8.6 — Understanding & Trust

MAVIS understands instead of following a script. The model decides what was meant and how to say it; code checks actions and keeps them safe. The vision behind this phase is [`DECISIONS.md` §48](DECISIONS.md#48-the-vision-for-mavis). Each step gets a hardware run before the next.

Done:

- [x] **DO/SAY** ([§44](DECISIONS.md#44-the-model-understands-the-code-checks-dosay)). One model call per sentence answers `DO <action>` or `SAY <reply>`, held to that form by a grammar. Arguments are held to their kind. Actions come from a catalogue in code, checked before they run.
- [x] **No phrase lists left** ([§46](DECISIONS.md#46-no-phrase-lists-left)). Confirmations, Sentinel questions, going quiet, looking back and remembering all go through the model as actions. High-risk actions are asked twice.
- [x] **No test history in the code** ([§47](DECISIONS.md#47-no-test-history-in-the-code)).

Next, in order:

- [ ] **1. Natural replies and a long conversation.** Every word MAVIS says comes from the model: actions report what happened and MAVIS says it its own way. The fixed reply strings go. Conversation is held over many turns; older turns are summarised, not dropped, and what matters is kept for later. Qwen2.5-3B becomes the default model.
  - [x] Actions speak in the model's words; what couldn't be done, and failures, are told by the model from the facts ([§49](DECISIONS.md#49-mavis-says-it-in-its-own-words)).
  - [ ] The Sentinel's answers, the gate's questions, "didn't catch that" and the mic warnings in the model's words; the name learned through the model.
  - [ ] A long conversation on Qwen2.5-3B, with older turns summarised.
- [ ] **2. The open action.** Beyond the catalogue, the model can propose its own command for anything the catalogue doesn't cover. Everything goes through the risk gate: harmless commands run, risky ones are explained in plain words and wait for a yes, root goes through `pkexec`, irreversible ones are asked twice. Then multi-step tasks: plan, act, read the result, carry on.
- [ ] **3. Learning from use.** From the owner's confirmed actions, stated facts and daily patterns, never from undone or corrected ones or other voices. Stored as data that can be read out and forgotten; nothing leaves the machine; the model's weights never change.
- [ ] **4. Voice and face for owner-only actions.** See the rules below. Enrollment is a separate command until the install script exists.
- [ ] **5. Files and projects.** Find and read files of any format, summarise and explain them, and build a project from a document with a plan, a time estimate and progress reports. Local on every machine, with optional cloud help (the user's own key) for large builds.
- [ ] **6. Install script.** Last, once everything else is done. One command: detects the hardware, picks and downloads models that fit, installs everything, enrolls the owner's voice and face, and sets MAVIS up. The hardware check lives in MAVIS, so it can suggest a better model if the machine changes.

Who can approve what:

| Anyone | Owner only |
|---|---|
| Media, opening and closing apps, volume, brightness, workspaces, search, questions, conversation, locking the screen | `sudo` or root, installing or removing software, system settings, users and services, anything irreversible, logging out, suspend, shut down, restart, deleting files |

Rules for voice and face:

- Enrolled for the owner during installation. Only templates are stored, locally; never recordings or photos. They can be re-enrolled or deleted at any time.
- Voice is checked in whatever the owner says at that moment, never a fixed phrase, so a recording of the enrollment can't unlock anything.
- The face check runs in the background with no camera window, only at owner-only moments, for a second or two. The camera light always comes on; MAVIS never hides it. Frames are compared and discarded straight away.
- Liveness is checked from natural movement; if unsure, MAVIS asks for a blink. If the owner still can't be confirmed, the system password (`pkexec`) is the fallback.
- Machines without a camera use voice alone.

Models by machine (chosen by the install script, changeable in `config/config.toml`):

| Machine | Language model | Speech recognition |
|---|---|---|
| No graphics card, 4–8 GB RAM | Qwen2.5 0.5B–1.5B on the processor | Whisper tiny or base |
| About 6 GB graphics memory | Qwen2.5-3B on the graphics card | Whisper small |
| 12 GB or more | Qwen2.5 7B–14B | Whisper medium |

---

## Phase 9 — Skills Platform

**Goal:** Modular capabilities. Each skill is a Rust crate or Python module with a manifest.

### 9.1 Plugin API

- [ ] **Manifest format** — `mavis-skill.toml`:
  ```toml
  [skill]
  name = "git"
  version = "0.1.0"
  required_tier = "Execute"
  capabilities = ["read_cwd", "exec_command"]
  rust_entrypoint = "mavis_skill_git::register"
  python_entrypoint = "mavis_skill_git_python::handler"
  ```
- [ ] **Lifecycle hooks** — `on_load`, `on_unload`, `on_intent`, `on_context_change`
- [ ] **Capability declarations** — Skills request capabilities; runtime denies if user hasn't granted permission tier
- [ ] **Version compatibility** — Semantic versioning for skills; runtime warns on incompatible API versions
- [ ] **Sandboxing** — Python skills run in separate process (like AI worker); Rust skills are in-process but capability-gated

### 9.2 Core Skills (shipped with MAVIS)

| Skill | Tier | Description |
|-------|------|-------------|
| `filesystem` | Execute | Read, write, move, search files; respects `$HOME` boundaries |
| `browser` | Read | Read active tab title/URL; open URLs; basic bookmark search |
| `git` | Execute | Status, commit, branch, log; dry-run by default |
| `vscode` | Read + Execute | Open files, read recent projects, run tasks |
| `email` | Ask | Read local Maildir / notmuch; draft replies (never send without confirm) |
| `calendar` | Read | Read local calendars; inject next-event into context |
| `spotify` / `media` | Ask | MPRIS control; play/pause/next; respects DND |
| `discord` | Ask | Read unread mentions; draft replies (never send without confirm) |

### 9.3 Skill Discovery

- [ ] Skills placed in `~/.config/mavis/skills/` or system path
- [ ] Runtime scans manifests on startup; registers event handlers
- [ ] Hot-reload in dev mode (`MAVIS_DEV=1`)

---

## Phase 10 — Automation, Proactive Intelligence & Wellness

**Goal:** MAVIS should assist *before* being asked. And care for the human behind the keyboard.

### 10.1 Rule Engine

- [ ] **Trigger types** — Time, event (window changed, file created), voice intent, context match
- [ ] **Condition language** — Simple JSON DSL: `{"and": [{"active_window": "code"}, {"time_after": "09:00"}]}`
- [ ] **Actions** — Call any skill capability; chainable
- [ ] **Scheduled automations** — Cron-like scheduling via Rust background task
- [ ] **Trigger-based automations** — React to events from the event bus
- [ ] **Conditional automations** — Complex boolean logic across context sources

### 10.2 Predictive Suggestions

- [ ] **Predict likely next action** — Markov chain / simple classifier on user action sequences
- [ ] **Suggest files/projects** — "You often open `notes.md` after `daily_standup.ics`; shall I open it?"
- [ ] **Offer automations** — "You've done this 5 times this week. Shall I create a shortcut?"
- [ ] **Detect repetitive workflows** — Pattern match on action sequences; suggest macro recording

### 10.3 Workflow Recording & Macro Execution

- [ ] User says "MAVIS, record this"
- [ ] Rust captures: window switches, clipboard changes, terminal commands (opt-in), file opens
- [ ] User says "MAVIS, stop recording; name it 'deploy workflow'"
- [ ] Saved as replayable automation script (Rust macro or shell script)
- [ ] **Macro execution** — Replay recorded workflows with variable substitution

### 10.4 Daily Intelligence

- [ ] **Daily briefing** — Morning TTS summary: calendar, weather (if opted in), overdue tasks, anomalies
- [ ] **End-of-day summary** — "Today you worked on X, committed Y, have Z unread emails. Good night."

### 10.5 Wellness Reminders

- [ ] **Water reminders** — Every 45 min of active computer use; gentle orb pulse + optional TTS
- [ ] **Meal reminders** — Based on learned schedule; "It's usually lunch time. Don't forget to eat."
- [ ] **Stretch reminders** — Every hour; "Your shoulders are probably tense. 30-second stretch?"
- [ ] **Sleep reminders** — Based on learned bedtime; orb dims, gentle nudge after threshold
- [ ] **Screen break reminders** — 20-20-20 rule prompt; optional screen dim
- [ ] All wellness features respect DND mode and can be snoozed or disabled per-category

---

## Phase 11 — Vision & Advanced UX

**Goal:** The orb sees the screen. Not to spy — to understand.

### 11.1 Vision

- [ ] **Screenshot pipeline** — Rust grabs frame (Wayland `screencopy` or `grim`); pipes to Python
- [ ] **OCR** — `easyocr` or `tesseract` (local, CPU); extract text from screen regions
- [ ] **Screenshot understanding** — Lightweight vision-language model (LLaVA / BakLLaVA, ~4 GB) for describing screen content
- [ ] **Object detection** — Lightweight YOLO or DETR model (~50 MB) for UI elements, windows, notifications
- [ ] **UI element detection** — Detect buttons, input fields, menus, dialogs for accessibility automation
- [ ] **Screen context** — Compact description of "what's on screen" injected into working memory

### 11.2 Advanced UX

- [ ] **Secondary monitor dashboard** — Optional `mavis_dashboard` binary; shows memory graph, recent actions, skill status, wellness stats
- [ ] **Rich notifications** — Orb expands briefly to show text + icon; auto-dismisses; never steals focus
- [ ] **Conversation history browser** — GTK/Rust app for searching past interactions; local-only, encrypted at rest
- [ ] **Mobile companion** — Optional lightweight Android/iOS app for remote status check and voice notes; syncs via local network only
- [ ] **Companion mode** — MAVIS runs in reduced-resource mode on secondary device; mirrors core state

---

## Phase 12 — Multi-Model, Cross-Platform & Personality Maturity

**Goal:** Future-proofing. Run anywhere, route intelligently. Become a consistent self.

### 12.1 Multi-Model

- [ ] **Local model registry** — `~/.config/mavis/models/` with manifests
- [ ] **Capability-based routing** — Planner chooses model by task:
  - Chat / reasoning -> Phi-3 / Llama-3
  - Code -> CodeLlama / DeepSeek-Coder
  - Vision -> LLaVA / BakLLaVA
  - Fast fallback -> Phi-3-mini for low-latency intents
- [ ] **Model benchmarking** — Automatic perplexity / latency benchmarks on user hardware; ranks models
- [ ] **Automatic model selection** — Pick best model per task based on benchmark scores and current resource availability *(choosing models to fit the machine is Phase 8.6 step 6)*
- [ ] **Cloud help** — moved to Phase 8.6 step 5: optional, off by default, the user's own key, for large builds.

### 12.2 Cross-Platform

- [ ] **Linux** — Primary target; Wayland + Niri optimized; X11 fallback
- [ ] **Windows** — `mavis_core` compiles with `windows` crate; Win32 API for window tracking, TTS via SAPI5, STT via Whisper.cpp
- [ ] **macOS** — `objc` / `cocoa` bindings for accessibility API; TTS via `say`; STT via Whisper.cpp
- [ ] **Platform traits** — `AudioCapture`, `WindowTracker`, `ClipboardReader`, `ScreenGrabber` — implemented per-platform

### 12.3 Personality Maturity

- [ ] **Consistent personality** — Personality is not a prompt hack; it's a persisted configuration (JSON) that shapes tone, humor level, verbosity, and boundaries. Survives model swaps.
- [ ] **User preference adaptation** — Track user reactions (accepted suggestions, snoozed reminders, interrupted TTS) to refine personality parameters over weeks
- [ ] **Conversation style evolution** — If user prefers brevity, MAVIS learns to default to 1-sentence responses; if user likes detail, expands naturally

---

## Appendices

### A. Architecture Constraints (Preserved Across All Phases)

1. **Rust primary runtime** — All system I/O, UI, audio, window tracking lives in Rust.
2. **Python AI worker only** — LLM inference, STT, embeddings, vision. Spawned by Rust, not assumed running.
3. **Context Engine is central** — All subsystems publish events; Context Engine maintains canonical state.
4. **Planner never executes** — Planner generates plans; Executor carries them out. Separation of strategy and action.
5. **Event bus** — in-process `tokio::sync::broadcast`; the Python worker over UDS + length-prefixed JSON. No HTTP, no gRPC, no cloud APIs in the core loop.
6. **Local-first, privacy-first** — No telemetry. Everything runs locally by default; cloud help is optional and off unless the user turns it on.
7. **Voice is interface, not identity** — MAVIS does not "become" a voice. The orb is the identity.
8. **Layered memory** — Working -> Recall -> Long-Term, with Episodic and Entities alongside. No single flat store. *(Permanent and Session were planned and removed 2026-09-19; see Phase 7.)*
9. **AI-agnostic** — Prompts and context format must work with Qwen, Phi-3, Llama, Mistral, etc.
10. **"Always present. Never intrusive."** — Every feature must pass this test before shipping.
11. **The model understands; code checks** — the model decides what was meant and chooses actions from the catalogue (Phase 8.6). Code checks every choice, and risk scores and Sentinel severity stay static: the model may never lower a score.
12. **Fail loud, degrade gracefully** — a missing microphone costs MAVIS its ears, not its life; a panicking subsystem is restarted and logged.

### B. Definition of Done for Each Phase

- [ ] All code passes `cargo check` / `cargo clippy` / `cargo test` *(as of 2026-11-25: 275 + 277 tests pass, clippy with warnings only; the pre-commit hook runs only `ruff`, so nothing enforces the Rust checks)*
- [ ] Python code passes `ruff` pre-commit
- [ ] Feature documented in `PHASES.md`, with the reasons in `DECISIONS.md`
- [ ] E2E test script exists and passes
- [ ] No regression in prior phase features
- [ ] Memory instruction updated if architecture changes

### C. Resource Budget (development machine: Ryzen 5, 16 GB RAM, RTX 4050 6 GB)

Other machines get models that fit them; see Phase 8.6, models by machine.

| Component | Max RAM | Max VRAM | Notes |
|-----------|---------|----------|-------|
| LLM (Qwen2.5-3B, Q4_K_M) | 3 GB | 3 GB | Fully on the graphics card |
| STT (faster-whisper small.en) | 1 GB | 0 GB | CPU-only, `int8` |
| Embeddings (MiniLM) | 300 MB | 0 GB | Not used — memory search is SQLite FTS5 (Phase 7) |
| Vision (YOLO / small DETR) | 200 MB | 2 GB | Optional; unload when idle |
| Rust runtime + UI | 200 MB | 0 GB | Orb, event bus, context engine |
| **Total (all loaded)** | **~8 GB** | **~6 GB** | Leaves headroom for user apps |
| **Idle (only Rust)** | **~200 MB** | **0 GB** | Models fully unloaded |

---

## Development rules

- One feature per branch, one branch per PR.
- Every Rust module gets a `cargo test` before it's done.
- Every Python module gets a `pytest` before it's done.
- Update `PHASES.md` before or alongside implementation.
- Credentials in `.env` only. Never in code.
- Dry-run mode must always work.
- Comments explain why, not what.
- Verify before trusting a "success" message.
- When you fix a bug, search for the same pattern elsewhere. Fix the class, not the instance.
- A regression test isn't proven until it has been seen failing against the old code.
- Record decisions in `DECISIONS.md` in the same change that makes them.
- Never block the async runtime.
- No circular dependencies.
- Privacy is default.
- No scripts: phrase lists, sample sentences, run logs and pre-written replies never decide what MAVIS does or says.
- Fix the ability behind a problem, not the one sentence that failed. Run history goes in `DECISIONS.md`, never in code.
- Test data is neutral: no real people, machines or session transcripts.

---

*Last updated: 2026-11-26*