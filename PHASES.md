# MAVIS — Project Phases

> **Tagline:** A persistent desktop-native AI companion. Not a chatbot.
>
> **Project goal:** Build a local-first, privacy-first desktop AI companion. Rust owns the runtime (~50 MB, always on). Python owns the AI (spawned on demand, killed when idle).
>
> **Repo:** `github.com/Manav-1200/MAVIS`
>
> **Stack:** Rust (tokio, minifb, serde, rusqlite, notify), Python 3.10+ (llama-cpp-python, faster-whisper, piper)

---

## Quick reference — phase overview

| Phase | Name | Core deliverable | Portfolio label | Status |
|-------|------|-----------------|-----------------|--------|
| 1 | Foundation | Rust runtime compiles. Async event bus. Orb window appears. Python worker reorganized. | Rust desktop app | ✅ Complete |
| 2 | Core Runtime | Context engine. Layered memory (SQLite). System integration. Orb states. Graceful shutdown. | Event-driven runtime | ✅ Complete |
| 3 | AI Worker | Rust ↔ Python bridge. Local LLM loads. First inference. Worker lifecycle. | Local AI pipeline | ✅ Complete |
| 4 | Integration | Voice wake → STT → LLM → TTS. Intent system. First automations. | Full voice companion | 🚧 In Progress |
| 5 | Polish | Performance. Packaging. systemd service. Daily driver. | Shippable product | Not started |
| 6 | Extras | Ideas that come up during build | — | Ongoing |

---

## Architecture

```
┌─────────────┐     ┌─────────────┐
│ Living Orb  │────▶│   Context   │
│   (Rust)    │     │   Engine    │
└─────────────┘     │   (Rust)    │
       ▲            └──────┬──────┘
       │                   │
       │            ┌──────▼──────┐
       │            │   Planner   │
       │            │   (Rust)    │
       │            └──────┬──────┘
       │                   │
       │            ┌──────▼──────┐
       │            │   Executor  │
       │            │   (Rust)    │
       │            └──────┬──────┘
       │                   │
       │            ┌──────▼──────┐
       └────────────│ AI Worker   │
                    │  (Python)   │
                    └─────────────┘
```

**Runtime split:**
- **`mavis_core` (Rust):** UI, event bus, context engine, memory, system integration. ~50 MB. Always on.
- **`mavis_worker` (Python):** AI inference, model weights, voice. Spawned on demand. Killed when idle.

**Protocol:** JSON over UDS (Unix domain socket). Not HTTP. Not gRPC.

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
- [x] `WorkingMemory` — in-memory `VecDeque<Event>` with intent/plan/state tracking
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

**Status:** ✅ COMPLETE (2026-08-10)

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
- [x] Crash recovery — restart once. Second crash within 60s → mark unavailable.
- [x] `MAVIS_PYTHON_PATH` env var for venv python

### 3.3 — Model loading
- [x] **Decision:** `llama-cpp-python` with quantized models (Q4_K_M)
- [x] Target: 3–4 GB model, 2–3 GB headroom on RTX 4050 6GB
- [x] Primary: `Phi-3-mini-4k-instruct-Q4_K_M.gguf` (~2.3 GB)
- [x] `mavis_worker/src/mavis/inference/engine.py` — `load_model()`, `generate()`, `chat()`, `get_memory_usage()`
- [x] First inference: "Hello MAVIS" → load → generate → orb `Thinking → Speaking`

### 3.4 — Prompt system
- [x] `mavis_worker/src/mavis/inference/prompts.py` — prompt templates
- [x] System prompt defines MAVIS personality
- [x] `build_chat_messages()` with working memory injection support
- [x] Context injection from working memory

### 3.5 — Phase 3 wrap-up
- [x] First request → worker spawns → model loads → response → orb animates. Under 10s.
- [x] Worker unloads after idle. VRAM reclaimed.
- [x] Crash recovery works.
- [x] Tag: `v0.3.0-ai-worker`

---

## Phase 4 — Integration

**Status:** 🚧 IN PROGRESS (2026-08-10 — ongoing)

**Goal:** Voice in, voice out. Intent classification. First automations.

### 4.1 — Speech-to-text
- [x] **Decision:** `faster-whisper` (not whisper.cpp). CPU int8 to avoid VRAM contention.
- [x] Audio capture via `cpal`. Energy-based VAD for speech segmentation.
- [x] Explicit ALSA device selection (`sysdefault:CARD=Generic`) to avoid silent "default" route.
- [x] `vad_filter=False` in faster-whisper — Rust VAD is single source of truth.
- [x] `min_speech_duration_ms=500` to filter noise bursts.
- [x] UDS protocol: `readexactly()` fix for partial socket reads on large audio payloads.
- [x] STT request timeout with retries (model load can be slow).

### 4.2 — Text-to-speech
- [x] **Decision:** `piper` via subprocess (`piper-tts` + `aplay`).
- [x] Async blocking `say()` — waits for playback to finish before returning.
- [x] Executor emits `UiStateChange("speaking")` before TTS and `idle` after.

### 4.3 — Voice loop & feedback prevention
- [x] E2E flow: speak → VAD detects → STT transcribes → Planner routes → LLM generates → Executor runs TTS → Amy speaks.
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

### 4.6 — Current blockers
- [ ] LLM regurgitates system prompt / example dialogues → **Fix:** Phi-3 manual chat template + stop tokens
- [ ] Working memory not injected into LLM prompts → **Fix:** ContextEngine → Planner enrichment
- [ ] STT echo/repetition ("Hi. Hi. Hi.") → **Fix:** Trim trailing silence before Whisper
- [ ] Idle unload not yet runtime verified with both models
- [ ] Niri push-to-talk hotkey deferred to Phase 5

### 4.7 — Phase 4 wrap-up checklist
- [x] STT engine (faster-whisper) integrated
- [x] cpal audio capture + energy VAD
- [x] UDS protocol fixed (`readexactly`)
- [x] Worker eager spawn
- [x] STT request timeout for model load
- [x] faster-whisper model cached locally
- [x] E2E voice loop wired (speak → hear response)
- [x] E2E voice loop verified clean (no self-triggering)
- [ ] Idle unload verified with both models
- [ ] LLM response quality fixed (no prompt regurgitation)
- [ ] Niri hotkey binding for push-to-talk (deferred)

---

## Phase 5 — Polish

**Goal:** Daily driver. Fast. Stable. Packaged. Someone else can install it.

### 5.1 — Performance
- [ ] Rust startup < 2s.
- [ ] Worker spawn + model load < 5s from cold.
- [ ] Rust memory < 200 MB resident.
- [ ] Worker VRAM < 5 GB.
- [ ] Event bus latency < 1 ms.
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

> Add ideas as they come up. Move into a phase above when committed.

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
