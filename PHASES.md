# MAVIS — Project Phases

&gt; **Tagline:** A persistent desktop-native AI companion. Not a chatbot.
&gt;
&gt; **Project goal:** Build a local-first, privacy-first desktop AI companion. Rust owns the desktop (~50 MB, always on). Python owns the AI (spawned on demand, killed when idle).
&gt;
&gt; **Portfolio value:** Each phase is a standalone milestone. Phase 1 = Rust desktop app. Phase 3 = local AI pipeline. Phase 5 = shippable product.
&gt;
&gt; **Repo:** `github.com/Manav-1200/MAVIS`
&gt;
&gt; **Stack:** Rust (tokio, winit/softbuffer, serde, rusqlite), Python 3.10+ (transformers, llama-cpp-python, fastapi)

---

## Quick reference — phase overview

| Phase | Name | Core deliverable | Portfolio label | Status |
|-------|------|-----------------|-----------------|--------|
| 1 | Foundation | Rust runtime compiles. Async event bus. Orb window appears. Python worker reorganized. | Rust desktop app | 🚧 In progress |
| 2 | Core Runtime | Context engine. Layered memory (SQLite). System integration. Orb states. | Event-driven runtime | Not started |
| 3 | AI Worker | Rust ↔ Python bridge. Local LLM loads. First inference. Worker lifecycle. | Local AI pipeline | Not started |
| 4 | Integration | Voice wake → STT → LLM → TTS. Intent system. First automations. | Full voice companion | Not started |
| 5 | Polish | Performance. Packaging. systemd service. Daily driver. | Shippable product | Not started |
| 6 | Extras | Ideas that come up during build | — | Ongoing |

---

## Phase 1 — Foundation

**Goal:** `mavis_core` compiles and runs. The event bus works. An orb window appears. The Python worker is reorganized from existing code.

**Why this order:** Everything hangs off the event bus. The orb is the only visual touchpoint — it has to exist before we worry about what it displays.

### 1.1 — Project scaffold

- [ ] Create the complete workspace structure:


MAVIS/
├── mavis_core/
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs
│       ├── lib.rs
│       ├── event_bus.rs
│       ├── context_engine.rs
│       ├── planner.rs
│       ├── executor.rs
│       ├── worker_bridge.rs
│       ├── memory/
│       │   ├── mod.rs
│       │   ├── manager.rs
│       │   ├── permanent.rs
│       │   ├── long_term.rs
│       │   ├── episodic.rs
│       │   ├── session.rs
│       │   └── working.rs
│       ├── ui/
│       │   ├── mod.rs
│       │   ├── orb.rs
│       │   └── states.rs
│       ├── system/
│       │   ├── mod.rs
│       │   ├── dbus.rs
│       │   ├── hotkeys.rs
│       │   └── watcher.rs
│       └── models/
│           ├── mod.rs
│           └── event.rs
├── mavis_worker/
│   ├── pyproject.toml
│   └── src/
│       └── mavis/
│           ├── init.py
│           ├── main.py
│           ├── main.py
│           ├── bootstrap.py
│           ├── app.py
│           ├── worker.py
│           ├── core/
│           │   ├── init.py
│           │   ├── config.py
│           │   ├── constants.py
│           │   ├── events.py
│           │   ├── lifecycle.py
│           │   ├── logger.py
│           │   └── paths.py
│           ├── inference/
│           │   ├── init.py
│           │   ├── engine.py
│           │   └── prompts.py
│           ├── voice/
│           │   ├── init.py
│           │   ├── stt.py
│           │   └── tts.py
│           └── skills/
│               ├── init.py
│               └── registry.py
├── config/
│   └── default.toml
├── data/
├── logs/
├── memory/
├── scripts/
│   ├── install.sh
│   └── uninstall.sh
├── .github/
│   └── workflows/
│       └── test.yml
├── .env.example
├── .gitignore
├── AI_CONTEXT.md
├── PHASES.md
├── README.md
├── CHANGELOG.md
└── LICENSE


- [x] **Decision:** `uv` for Python, `ruff` for format/lint, `pre-commit` for git hooks.
- [ ] `mavis_core/Cargo.toml` — tokio, serde, serde_json, uuid, chrono, rusqlite, dirs, log, env_logger, anyhow, thiserror.
- [ ] `mavis_worker/pyproject.toml` — torch, transformers, numpy, fastapi, uvicorn.

### 1.2 — Event Bus

- [ ] `event_bus.rs` — `Event` struct and `EventType` enum.
- [ ] `EventBus` using `tokio::sync::broadcast`, capacity 256.
- [ ] Methods: `new()`, `publish()`, `subscribe()`.
- [ ] `models/event.rs` — shared data structures, re-exported.

### 1.3 — Entry point

- [ ] `main.rs` — tokio async main.
- [ ] Init logging. Create event bus. Create context engine stub. Create orb stub.
- [ ] Block on `ctrl_c` for clean shutdown.
- [ ] Log lifecycle points.

### 1.4 — Orb UI stub

- [ ] `ui/orb.rs` — `Orb` struct with winit window.
- [ ] `ui/states.rs` — `OrbState` enum.
- [ ] Borderless, always-on-top, ~128x128 px, bottom-right.
- [ ] **Decision:** winit + softbuffer for Phase 1. Raw Wayland in Phase 6.

### 1.5 — Subsystem stubs

- [ ] `context_engine.rs` — stub with `new()` and `process_event()`.
- [ ] `planner.rs` — stub with `plan()` signature.
- [ ] `executor.rs` — stub with `execute()` signature.
- [ ] `worker_bridge.rs` — stub with `spawn()`, `send()`, `recv()`.
- [ ] `memory/manager.rs` — `MemoryManager` facade.
- [ ] `memory/working.rs` — in-memory store.
- [ ] `memory/permanent.rs`, `long_term.rs`, `episodic.rs`, `session.rs` — SQLite stubs.
- [ ] `system/dbus.rs`, `hotkeys.rs`, `watcher.rs` — stubs.

### 1.6 — Python worker reorganization

- [ ] Move existing `core/` modules into `mavis_worker/src/mavis/core/`.
- [ ] `app.py` — FastAPI server or stdin/stdout loop.
- [ ] `worker.py` — process lifecycle.
- [ ] `inference/engine.py` and `prompts.py` — stubs.
- [ ] `voice/stt.py`, `voice/tts.py` — stubs.
- [ ] `skills/registry.py` — stub.

### 1.7 — Phase 1 wrap-up

- [ ] `cargo check` passes clean.
- [ ] `cargo run` opens orb window, stays alive until `Ctrl+C`.
- [ ] `cargo test` passes for event bus and models.
- [ ] `pip install -e .` succeeds in `mavis_worker/`.
- [ ] `python -m mavis` starts and stays alive.
- [ ] Tag: `git tag v0.1.0-foundation`

---

## Phase 2 — Core Runtime

**Goal:** Context engine runs. Memory persists to SQLite. System events flow. Orb shows real state transitions.

### 2.1 — Context Engine

- [ ] Maintains `WorkingMemory`.
- [ ] `process_event()` routes events to working memory.
- [ ] Context window limited to ~4k tokens. Old context promoted to Session.
- [ ] `get_context_for_worker()` serializes working memory to JSON.

### 2.2 — Memory layers

- [ ] `PermanentStore` — SQLite key-value. Identity, core preferences.
- [ ] `LongTermStore` — SQLite. Learned patterns.
- [ ] `EpisodicStore` — SQLite. Events with timestamps.
- [ ] `SessionStore` — SQLite. Conversation thread. Cleared on restart.
- [ ] `WorkingMemory` — in-memory only.
- [ ] `MemoryManager` — single interface. Promotion/demotion logic.

### 2.3 — Planner

- [ ] Receives intent from Context Engine.
- [ ] Decomposes into `Plan` (sequence of `Action`s).
- [ ] Action types: `QueryAI`, `LaunchApp`, `OpenFile`, `SearchWeb`, `SetReminder`, `NotifyUser`.
- [ ] Never executes. Returns plan to Executor.

### 2.4 — Executor

- [ ] Receives plan from Planner.
- [ ] `QueryAI` → spawns worker via `WorkerBridge`.
- [ ] `LaunchApp` / `OpenFile` → `std::process::Command`.
- [ ] `SearchWeb` → opens browser.
- [ ] `SetReminder` → tokio timer, publishes event when fired.
- [ ] `NotifyUser` → DBus notification.
- [ ] Reports results as `ActionComplete` events.

### 2.5 — System integration

- [ ] `system/dbus.rs` — notifications, media keys, power.
- [ ] `system/watcher.rs` — filesystem watchers.
- [ ] `system/hotkeys.rs` — global shortcuts (`Super+M` to wake).
- [ ] All publish to Event Bus. No direct coupling.

### 2.6 — Orb state machine

- [ ] States: `Idle`, `Listening`, `Thinking`, `Speaking`, `Working`, `Notification`, `Error`, `Sleeping`.
- [ ] Transitions driven by Event Bus.
- [ ] Simple color/opacity animations.
- [ ] "Never intrusive" — fades on typing, respects focus.

### 2.7 — Error handling

- [ ] Subsystem isolation — one crash cannot kill runtime.
- [ ] Graceful degradation — AI worker fails → orb shows error, runtime stays alive.
- [ ] `anyhow` for app errors, `thiserror` for library errors.

### 2.8 — Phase 2 wrap-up

- [ ] Hotkey wakes MAVIS. Orb transitions `Idle → Listening`.
- [ ] File changes appear in working memory.
- [ ] Memory persists across restarts.
- [ ] Planner decomposes intent. Executor launches test app.
- [ ] `cargo test` passes.
- [ ] Tag: `git tag v0.2.0-core-runtime`

---

## Phase 3 — AI Worker

**Goal:** Rust talks to Python. Local quantized LLM loads in 6GB VRAM. First inference. Worker lifecycle managed.

### 3.1 — Rust–Python bridge

- [ ] **Decision:** JSON over stdin/stdout (newline-delimited). Not HTTP. Not gRPC.
- [ ] `worker_bridge.rs` — `WorkerBridge` struct.
- `spawn()`, `send()`, `recv()`, `health_check()`, `kill()`.
- [ ] Python side — `app.py` reads stdin, parses JSON, routes to inference, writes stdout.
- [ ] Timeout: 30s default.

### 3.2 — Worker lifecycle

- [ ] Lazy load — spawns on first request.
- [ ] Health check every 30s. Restart if dead.
- [ ] Idle timeout — kill after 5 min. Reclaim VRAM.
- [ ] Crash recovery — restart once. Second crash within 60s → mark unavailable.

### 3.3 — Model loading

- [ ] **Decision:** `llama-cpp-python` with quantized models (Q4_K_M / Q5_K_M).
- [ ] Target: 3–4 GB model, 2–3 GB headroom.
- [ ] Candidates:
- `Llama-3.1-8B-Instruct-Q4_K_M.gguf` (~4.5 GB) — primary.
- `Phi-3-mini-4k-instruct-Q4_K_M.gguf` (~2.3 GB) — fallback.
- [ ] `inference/engine.py` — `load_model()`, `generate()`, `get_memory_usage()`.
- [ ] First inference: "Hello MAVIS" → load → generate → orb `Thinking → Speaking`.

### 3.4 — Prompt system

- [ ] `inference/prompts.py` — prompt templates.
- [ ] System prompt defines MAVIS personality.
- [ ] `build_chat_prompt()`, `build_action_prompt()`.
- [ ] Context injection from working memory and permanent store.

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

- [ ] Raw Wayland integration (replace winit).
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