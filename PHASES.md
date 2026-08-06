# MAVIS — Project Phases

&gt; **Tagline:** A persistent desktop-native AI companion. Not a chatbot.
&gt;
&gt; **Project goal:** Build a local-first, privacy-first desktop AI companion. Rust owns the runtime (~50 MB, always on). Python owns the AI (spawned on demand, killed when idle).
&gt;
&gt; **Repo:** `github.com/Manav-1200/MAVIS`
&gt;
&gt; **Stack:** Rust (tokio, minifb, serde, rusqlite, notify), Python 3.10+ (transformers, llama-cpp-python)

---

## Quick reference — phase overview

| Phase | Name | Core deliverable | Portfolio label | Status |
|-------|------|-----------------|-----------------|--------|
| 1 | Foundation | Rust runtime compiles. Async event bus. Orb window appears. Python worker reorganized. | Rust desktop app | ✅ Complete |
| 2 | Core Runtime | Context engine. Layered memory (SQLite). System integration. Orb states. Graceful shutdown. | Event-driven runtime | ✅ Complete |
| 3 | AI Worker | Rust ↔ Python bridge. Local LLM loads. First inference. Worker lifecycle. | Local AI pipeline | 🚧 Next |
| 4 | Integration | Voice wake → STT → LLM → TTS. Intent system. First automations. | Full voice companion | Not started |
| 5 | Polish | Performance. Packaging. systemd service. Daily driver. | Shippable product | Not started |
| 6 | Extras | Ideas that come up during build | — | Ongoing |

---

## Phase 1 — Foundation

**Status:** ✅ COMPLETE (2026-08-06)

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
- [x] `Ctrl+C` triggers `bus.close()` → all receivers wake, concurrent join with 5s timeout
- [x] Clean shutdown under 1 second

### 1.4 — Orb UI
- [x] `ui/orb.rs` — `Orb` struct, dedicated render thread
- [x] `ui/states.rs` — `OrbState` enum: Idle, Listening, Thinking, Speaking, Working, Error, Asleep
- [x] 80×80 px, borderless, transparent, always-on-top
- [x] Draggable (click-and-drag to reposition)
- [x] Soft pulsing circle with smoothstep gradient, state-reactive colors
- [x] **Decision:** `minifb` for Phase 1–2. Raw Wayland in Phase 6.

### 1.5 — Subsystem stubs
- [x] `context_engine.rs` — stub with `new()` and `process_event()`
- [x] `planner.rs` — stub with `run()` signature
- [x] `executor.rs` — stub with `run()` signature
- [x] `worker_bridge.rs` — UDS client stub
- [x] `memory/manager.rs` — `MemoryManager` facade
- [x] `memory/working.rs` — in-memory store
- [x] `memory/permanent.rs`, `episodic.rs` — SQLite stubs
- [x] `memory/long_term.rs`, `session.rs` — empty stubs
- [x] `system/dbus.rs`, `hotkeys.rs`, `watcher.rs` — empty stubs

### 1.6 — Python worker reorganization
- [x] `core/config.py` — TOML loading, validation, dot-notation `get()`
- [x] `core/logger.py` — logging setup
- [x] `core/events.py` — in-process EventBus for Python side
- [x] `bootstrap.py` — creates dirs, loads config, starts `MavisApp`
- [x] `worker.py` — asyncio UDS server at `/tmp/mavis_worker.sock`, lazy AI imports, responds with `[STUB]`

### 1.7 — Phase 1 wrap-up
- [x] `cargo check` passes clean
- [x] `cargo run` opens orb window, stays alive until `Ctrl+C`
- [x] `cargo test` passes for event bus and models
- [x] `pip install -e ./mavis_worker` succeeds
- [x] `python -m mavis.worker` starts UDS server
- [x] Tag: `v0.1.0-foundation`

---

## Phase 2 — Core Runtime

**Status:** ✅ COMPLETE (2026-08-06)

**Goal:** Context engine routes events. Memory persists to SQLite. System events flow. Orb shows real state transitions. Executor runs actions.

### 2.1 — Context Engine
- [x] Maintains `WorkingMemory` (50-event context window)
- [x] `process_event()` routes all `EventType` variants
- [x] Auto-persists `UserIntent`, `ActionComplete`, `PlanReady` to EpisodicStore
- [x] Routes `WorkerResponse` → `ContextUpdate` or passes to Planner
- [x] **Fix:** Removed double-publish of `PlanReady` (was causing duplicate execution)

### 2.2 — Memory layers
- [x] `PermanentStore` — SQLite key-value: `get/set/delete/list`
- [x] `EpisodicStore` — SQLite event log: `record/recent/search`
- [x] `WorkingMemory` — in-memory `VecDeque&lt;Event&gt;` with intent/plan/state tracking
- [x] `LongTermMemory` / `SessionStore` — stubs (Phase 6)
- [x] `MemoryManager` — owns all layers behind `RwLock`/`Mutex`

### 2.3 — Planner
- [x] Listens for `UserIntent` → generates `WorkerRequest`
- [x] Listens for `WorkerResponse` (type "plan") → emits `PlanReady`
- [x] Never executes. Returns plan to Executor via Event Bus.

### 2.4 — Executor
- [x] Receives `PlanReady` → parses flexible payloads (array, object with `actions`, single action)
- [x] Action types: `shell`, `app`, `notify`, `say`, `system`
- [x] `shell` → `sh -c` with stdout/stderr capture
- [x] `app` → `Command::spawn` with `xdg-open` fallback for URLs/paths
- [x] `notify` → `notify-send`
- [x] `say` → `spd-say` / `espeak` fallback / log-only
- [x] `system` → delegates to `SystemAction` event for DBusIntegration
- [x] Emits `ActionComplete` after each action
- [x] Emits `UiStateChange` (working → idle/error) for Orb feedback
- [x] Stops plan execution on first failure

### 2.5 — System integration
- [x] `system/dbus.rs` — `DbusIntegration`: listens for `SystemAction` events
  - `notify` → `notify-send`
  - `media_play_pause/next/previous` → `playerctl` / `dbus-send` fallback
  - `brightness_up/down` → `brightnessctl`
  - `volume_up/down/mute` → `pactl`
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
- [x] Planner decomposes intent → Executor runs actions
- [x] `cargo test` passes (7 tests)
- [x] Graceful shutdown under 1 second
- [x] Tag: `v0.2.0-core-runtime`

---

## Phase 3 — AI Worker

**Status:** 🚧 NEXT

**Goal:** Replace `[STUB]` with real model loading. Local quantized LLM in 6GB VRAM. First inference. Worker lifecycle managed.

### 3.1 — Rust–Python bridge
- [x] JSON over UDS (Unix domain socket). Not HTTP. Not gRPC.
- [x] `worker_bridge.rs` — `WorkerBridge` struct: `run()`, forwards `WorkerRequest`, publishes `WorkerResponse`
- [x] Python side — `worker.py` reads UDS, parses JSON, routes to inference, writes response
- [ ] Timeout handling on requests
- [ ] Health check / heartbeat

### 3.2 — Worker lifecycle
- [ ] Lazy load — spawns on first request
- [ ] Health check every 30s. Restart if dead.
- [ ] Idle timeout — kill after 5 min. Reclaim VRAM.
- [ ] Crash recovery — restart once. Second crash within 60s → mark unavailable.

### 3.3 — Model loading
- [ ] **Decision:** `llama-cpp-python` with quantized models (Q4_K_M / Q5_K_M)
- [ ] Target: 3–4 GB model, 2–3 GB headroom on RTX 4050 6GB
- [ ] Candidates:
  - `Llama-3.1-8B-Instruct-Q4_K_M.gguf` (~4.5 GB) — primary
  - `Phi-3-mini-4k-instruct-Q4_K_M.gguf` (~2.3 GB) — fallback
- [ ] `mavis_worker/src/mavis/inference/engine.py` — `load_model()`, `generate()`, `get_memory_usage()`
- [ ] First inference: "Hello MAVIS" → load → generate → orb `Thinking → Speaking`

### 3.4 — Prompt system
- [ ] `mavis_worker/src/mavis/inference/prompts.py` — prompt templates
- [ ] System prompt defines MAVIS personality
- [ ] `build_chat_prompt()`, `build_action_prompt()`
- [ ] Context injection from working memory and permanent store

### 3.5 — Phase 3 wrap-up
- [ ] First request → worker spawns → model loads → response → orb animates. Under 10s.
- [ ] Worker unloads after idle. VRAM reclaimed.
- [ ] Crash recovery works.
- [ ] Tag: `git tag v0.3.0-ai-worker`

---

## Phase 4 — Integration

**Goal:** Voice in, voice out. Intent classification. First automations.

### 4.1 — Voice wake word
- [ ] **Decision:** `porcupine` (picovoice). Custom wake word "Hey MAVIS".
- [ ] Runs in separate thread, always listening.
- [ ] On wake: publish `SystemWake`. Orb `Idle → Listening`.

### 4.2 — Speech-to-text
- [ ] **Decision:** `whisper.cpp` via subprocess. CUDA-enabled.
- [ ] Audio capture via `cpal`.
- [ ] VAD (`silero-vad` or `webrtc-vad`) detects end of speech.
- [ ] Flow: wake → record → VAD silence → whisper.cpp → transcript → `UserIntent`.

### 4.3 — Intent parser
- [ ] Rule-based classification: `Query`, `Action`, `Automation`, `System`, `Conversation`.
- [ ] Routes to Planner by intent type.

### 4.4 — Text-to-speech
- [ ] **Decision:** `piper`. Fallback: `coqui-tts`.
- [ ] Runs in Python worker. Audio playback via `cpal` in Rust.
- [ ] Orb `Thinking → Speaking` while audio plays.

### 4.5 — First automations
- [ ] "Open Firefox" → `LaunchApp`.
- [ ] "Open my notes" → `OpenFile`.
- [ ] "Search for ..." → `SearchWeb`.
- [ ] "Remind me in 10 minutes" → `SetReminder`.
- [ ] All through Planner → Executor.

### 4.6 — Phase 4 wrap-up
- [ ] "Hey MAVIS, open Firefox" → full pipeline under 5 seconds.
- [ ] Orb animates through all states.
- [ ] Tag: `git tag v0.4.0-voice-integration`

---

## Phase 5 — Polish

**Goal:** Daily driver. Fast. Stable. Packaged. Someone else can install it.

### 5.1 — Performance
- [ ] Rust startup &lt; 2s.
- [ ] Worker spawn + model load &lt; 5s from cold.
- [ ] Rust memory &lt; 200 MB resident.
- [ ] Worker VRAM &lt; 5 GB.
- [ ] Event bus latency &lt; 1 ms.
- [ ] Profile with `cargo flamegraph`.

### 5.2 — Error boundaries
- [ ] Kill worker mid-inference → runtime recovers.
- [ ] Kill STT mid-transcription → runtime recovers.
- [ ] Kill TTS mid-speech → runtime recovers.
- [ ] Orb stays alive. No crash kills the window.
- [ ] AI unavailable → "I'm having trouble thinking right now."

### 5.3 — Packaging
- [ ] `scripts/install.sh` — dirs, permissions, systemd user service.
- [ ] `scripts/uninstall.sh` — clean removal.
- [ ] systemd service: `mavis.service`. Restart on failure. Logs to journal.
- [ ] `.desktop` entry and orb icon.
- [ ] Release profile: `opt-level = 3`, `lto = true`, `strip = true`.

### 5.4 — CI and testing
- [ ] `.github/workflows/test.yml` — `cargo test`, `pytest`, `cargo clippy`, `cargo fmt --check`.
- [ ] Coverage targets: 70% Rust, 70% Python.
- [ ] Integration test: spawn worker, send request, verify response.

### 5.5 — Documentation
- [ ] `README.md` — architecture diagram, install, demo GIF.
- [ ] User guide in README — how to talk to MAVIS, customize `config.toml`.
- [ ] Troubleshooting — worker won't load, orb missing, audio broken, hotkeys failing.

### 5.6 — Phase 5 wrap-up
- [ ] Auto-starts on login.
- [ ] Survives full day without restart.
- [ ] Memory under budget.
- [ ] Tag: `git tag v1.0.0-daily-driver`
- [ ] **Portfolio Project 1 — complete product.**

---

## Phase 6 — Ideas backlog

&gt; Add ideas as they come up. Move into a phase above when committed.

- [ ] Raw Wayland integration (replace minifb).
- [ ] Niri workspace awareness.
- [ ] Emotion system for orb animations.
- [ ] Plugin system with permissions.
- [ ] Skill registry (natural language matching).
- [ ] Semantic search over long-term memory (vector embeddings).
- [ ] Predictive context (time, calendar, activity based).
- [ ] Multiple AI providers (Ollama, OpenAI, Anthropic fallback).
- [ ] Companion display mode (secondary monitor dashboard).
- [ ] Mobile companion app.
- [ ] Self-improving workflow suggestions.
- [ ] Model versioning and rollback.
- [ ] Config profiles (`work.toml`, `gaming.toml`).

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
- Never block the async runtime.
- No circular dependencies.
- Privacy is default.