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
| 1 | Foundation | Rust runtime compiles. Async event bus. Orb window appears. Python worker reorganized. | Rust desktop app | :white_check_mark: Complete |
| 2 | Core Runtime | Context engine. Layered memory (SQLite). System integration. Orb states. Graceful shutdown. | Event-driven runtime | :white_check_mark: Complete |
| 3 | AI Worker | Rust <-> Python bridge. Local LLM loads. First inference. Worker lifecycle. | Local AI pipeline | :white_check_mark: Complete |
| 4 | Integration | Voice wake -> STT -> LLM -> TTS. Intent system. First automations. | Full voice companion | :white_check_mark: Complete |
| 5 | Interaction Polish | TTS queue, interruption, session recovery, personality foundation. | Daily polish | :white_check_mark: Complete |
| 6 | Context Awareness | Active window, clipboard, browser, IDE, terminal, calendar. | Companion senses | Not started |
| 7 | Memory & Learning | Episodic -> long-term pipeline, semantic recall, vector embeddings, routine detection. | Persistent memory | Not started |
| 8 | Safety & Permissions | 5-tier permission model, risk scoring, audit log, dry-run, rollback. | Trust layer | Not started |
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
       |            +------v------+
       |            |   Planner   |
       |            |   (Rust)    |
       |            +------+------+
       |                   |
       |            +------v------+
       |            |   Executor  |
       |            |   (Rust)    |
       |            +------+------+
       |                   |
       |            +------v------+
       +------------| AI Worker   |
                    |  (Python)   |
                    +-------------+
```

**Runtime split:**
- **`mavis_core` (Rust):** UI, event bus, context engine, memory, system integration. ~50 MB. Always on.
- **`mavis_worker` (Python):** AI inference, model weights, voice. Spawned on demand. Killed when idle.

**Protocol:** JSON over UDS (Unix domain socket). Not HTTP. Not gRPC.

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
- [x] Draggable (click-and-drag to reposition)
- [x] Soft pulsing circle with smoothstep gradient, state-reactive colors
- [x] **Decision:** `minifb` for Phase 1-2. Raw Wayland in Phase 6.

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

**Status:** :white_check_mark: COMPLETE (2026-08-06)

**Goal:** Context engine routes events. Memory persists to SQLite. System events flow. Orb shows real state transitions. Executor runs actions.

### 2.1 — Context Engine
- [x] Maintains `WorkingMemory` (50-event context window)
- [x] `process_event()` routes all `EventType` variants
- [x] Auto-persists `UserIntent`, `ActionComplete`, `PlanReady` to EpisodicStore
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
  - `brightness_up/down` -> `brightnessctl`
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
- [x] Tag: `v0.2.0-core-runtime`

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
- [x] `vad_filter=False` in faster-whisper — Rust VAD is single source of truth.
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
- [ ] Fan noise robust filtering (may need silero-vad)
- [ ] Niri hotkey binding for push-to-talk (deferred to Phase 5)

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
- [x] `MAVIS_AUDIO_DEVICE` env var respected for output (`--device` for pw-play/paplay)

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
- [ ] **Push-to-talk / active listening** — **skipped by decision, not abandoned.** No hotkey is bound; passive always-on listening is the sole interaction model, matching "always present, never intrusive." Candidate for revisiting if a hotkey (e.g. repurposing the laptop's unused Copilot key) gets bound later.

### 5.8 — Full E2E Verification (2026-09-02)
Real bugs found through live voice testing on target hardware — not simulated, not assumed fixed from code review alone:
- [x] **TTS echo feedback loop** — root cause and fix under 5.6.
- [x] **Whisper hallucination on near-silence** — the model invents fluent filler ("thank you for watching", "I don't know what I'm talking about") with *high* confidence on quiet ambient noise, bypassing the confidence gate entirely. Added `HALLUCINATION_DENYLIST` in `stt/engine.py`, with whitespace/apostrophe normalization (segment-join artifacts and curly quotes were breaking exact-match comparisons on the first attempt).
- [x] **Low-confidence drops were a black box** — logging now includes the actual dropped text, not just the score, so borderline rejections are debuggable instead of silent.
- [x] **"MAVIS" misheard as baby/movies/moose** — added `initial_prompt="MAVIS"` to bias the Whisper decoder toward the rare proper noun. Measurable improvement (4/6 correct in one test session), not a full fix — expected limitation of a quantized small model on an uncommon name.
- [x] **Worker logs arriving late / out of order** — Python's stdout was block-buffered when piped (no `PYTHONUNBUFFERED`); prints without explicit `flush=True` queued for seconds before appearing, which had been actively misleading live debugging all session.
- [x] **Prompt leakage via paraphrase** — tightening `SYSTEM_PROMPT` rule 13's wording did not work; the model swapped "guiding principles" for "my guidance" and leaked the same way. Real fix: deterministic phrase-match in `planner.rs` (`is_meta_instruction_question`) that bypasses the LLM entirely for meta-questions about its own instructions, returning a fixed deflection instead. Verified 3/3 on real hardware, including against a mis-transcribed "Movis, do you have any system prompt?".
- [x] **Session recovery** — verified working across two consecutive restarts; correct user name persisted both times.
- [x] **Model path / GPU layer config drift** — `WorkerServer` was constructing `LlamaEngine()` with no arguments, silently ignoring `config.toml` entirely. Now wired through safely — a configured model path is only honored if that file actually exists on disk, otherwise falls back to the existing auto-detection, so a stale/wrong config value can never break a working setup.

**Known gap surfaced, not fixed (carried into Phase 6):** active-window and clipboard capture already run continuously (`ContextEngine: context injected` fires every ~2s) but that data never reaches the LLM — `build_working_memory()` in `planner.rs` doesn't read `active_window` or `last_clipboard`. This is exactly Phase 6's first two planned context sources; the capture half was already built, the wiring wasn't.

**Confirmed absent:** no vision/screen-reading capability exists anywhere in the pipeline. `capture_focused()` is defined in `platform/linux.rs` but never called by anything, and the loaded LLM (Phi-3-mini) is text-only regardless.

---

## Phase 6 — Context Awareness Foundation

**Goal:** The companion must know what the user is doing, not just what they are saying. All context sources are **opt-in per-tier** (see Phase 8).

| Source | Rust or Python | Implementation |
|--------|---------------|----------------|
| Active window | Rust | `zbus` + `org.gnome.Shell` / `niri` IPC / `wlr-foreign-toplevel-management` / `xdotool` |
| Clipboard | Rust | `wl-clipboard` listener; read-only by default; hashed content |
| Browser awareness | Rust + Python | Native messaging host or `xdotool` title polling; extract domain/title |
| IDE awareness | Rust | Window title regex (`code`, `nvim`, `emacs`); project path from cwd |
| Terminal awareness | Rust | Detect terminal window; capture last command via `PROMPT_COMMAND` hook (opt-in) |
| Workspace / Project | Rust | Track current working directory of focused terminal/IDE |
| Calendar / Time context | Python | Read local `.ics` or `calcurse` export; inject "next meeting in 15 min" into context |

- [ ] **Context snapshot** — Compact JSON blob injected into `working_memory_snapshot` every 5 s
- [ ] **Privacy gate** — Each source has a `ContextSource` permission tier; default = off
- [ ] **Cross-platform abstraction** — `PlatformProvider` trait with Linux/Windows/macOS implementations

---

## Phase 7 — Memory & Learning Layer

**Goal:** Move from session-scoped working memory to persistent, growing memory.

### 7.1 Intelligent Memory Pipeline

```
Working Memory (session) ---> Episodic (sqlite, 30 days) ---> Long-Term (sqlite + embeddings)
                                    |                              |
                                    v                              v
                            Importance scoring              Consolidation (nightly)
                            (LLM rates 1-10)                (summarize, compress, embed)
```

- [ ] **Importance scoring** — After each interaction, lightweight LLM call rates memory importance
- [ ] **Episodic store** — SQLite table: `(timestamp, role, content, importance, tags_json)`
- [ ] **Forgetting / decay** — Episodic entries below threshold importance auto-purge after 30 days
- [ ] **Automatic summaries** — Nightly cron-like task (Rust scheduler) compresses high-importance episodes into summaries
- [ ] **Long-term consolidation** — Summaries promoted to Long-Term memory with vector embeddings
- [ ] **Context compression** — When working memory grows too large, older entries are compressed into summary bullets before eviction
- [ ] **Episodic replay** — Ability to reconstruct "what happened on Tuesday afternoon" from timestamped episodic chain

### 7.2 Advanced Memory

- [ ] **Vector embeddings** — `sentence-transformers` (all-MiniLM-L6-v2, local, CPU, ~80 MB)
- [ ] **FAISS / hnswlib** — Local vector index for fast similarity search
- [ ] **Semantic recall** — Before Planner generates a plan, search episodic + long-term for semantic matches to current intent + context
- [ ] **Memory graph** — Lightweight entity extraction (spaCy or regex) linking people, projects, files, concepts in a navigable graph
- [ ] **Relationship graph** — Track relationships between entities over time

### 7.3 Learning Engine

- [ ] **Routine detection** — Time-series pattern matching on user actions (e.g., opens Spotify at 9 AM)
- [ ] **Preferred apps** — Track most-launched applications per context (work hours vs. evening)
- [ ] **Learn coding schedule** — Detect when user typically codes; pre-warm context with relevant projects
- [ ] **Learn workflows** — Recognize recurring sequences of actions
- [ ] **Learn frequently used commands** — Build per-project command history; suggest completions
- [ ] **Adapt suggestions over time** — If user consistently rejects a suggestion type, down-weight it in Planner scoring

---

## Phase 8 — Safety & Permission System

**Goal:** Local-first does not mean reckless. Every capability is gated.

### 8.1 Permission Tiers

| Tier | Description | Examples |
|------|-------------|----------|
| `Read` | Observe only | Window title, clipboard hash, file listing |
| `Notify` | Alert user | "You have a meeting in 5 min" |
| `Ask` | Propose action, wait for confirmation | "Shall I open your daily notes?" |
| `Execute` | Run command / modify file | `git commit`, `mv`, `rm` |
| `Administrator` | Destructive or system-wide | `pacman -Syu`, partition ops, network changes |

- [ ] **Per-plugin / per-skill permissions** — Each skill declares its required tier; user grants per-skill
- [ ] **Per-skill permissions** — Granular control: `git` may have `Execute` while `browser` only has `Read`
- [ ] **Dry-run mode** — `Execute` tier commands are echoed to user before running; user must voice-confirm
- [ ] **Audit log** — Append-only SQLite log: `(timestamp, skill, action, args, user_confirmed, risk_score)`
- [ ] **Confirmation prompts** — Visual + voice confirmation for any action above `Notify` tier

### 8.2 Safety Layer

- [ ] **Command validation** — Static regex / deny-list for dangerous commands (`rm -rf /`, `mkfs`, `dd if=/dev/zero`)
- [ ] **Risk scoring** — Static analysis + heuristic scoring: file deletion = high risk; file creation = low; network call = medium
- [ ] **LLM validation** — Second-pass LLM call rates risk 1-10 for any `Execute` action; blocks if >= 8 without `Administrator` tier
- [ ] **Rollback where possible** — File operations create `.mavis-backup/` snapshots before destructive actions; allow undo within 5 minutes
- [ ] **Confirmation for destructive actions** — Any action with risk >= 5 requires explicit user confirmation; no silent execution

### 8.3 Confirmation Flow

```
Executor proposes action -> Risk score computed
    -> If score < 3: execute silently (respects "Never intrusive")
    -> If score 3-7: TTS "Shall I <action>?" -> wait 5 s for "yes" / orb tap
    -> If score >= 8: TTS "This requires administrator permission. Confirm?" -> require explicit "yes, administrator"
```

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
- [ ] **Automatic model selection** — Pick best model per task based on benchmark scores and current resource availability
- [ ] **Cloud fallback** — Optional, opt-in, privacy-preserving (no conversation text sent; only anonymized embeddings if needed)

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
5. **Event bus** — UDS + JSON. No HTTP, no gRPC, no cloud APIs in core loop.
6. **Local-first, privacy-first** — No telemetry. No cloud STT/TTS by default. All models local.
7. **Voice is interface, not identity** — MAVIS does not "become" a voice. The orb is the identity.
8. **Layered memory** — Permanent -> Long-Term -> Episodic -> Session -> Working. No single flat store.
9. **AI-agnostic** — Prompts and context format must work with Phi-3, Llama, Mistral, etc.
10. **"Always present. Never intrusive."** — Every feature must pass this test before shipping.

### B. Definition of Done for Each Phase

- [ ] All code passes `cargo check` / `cargo clippy` / `cargo test`
- [ ] Python code passes `ruff` pre-commit
- [ ] Feature documented in `docs/phase-N.md`
- [ ] E2E test script exists and passes
- [ ] No regression in prior phase features
- [ ] Memory instruction updated if architecture changes

### C. Resource Budget (Target Hardware: Ryzen 5, 16 GB RAM, RTX 4050 6 GB)

| Component | Max RAM | Max VRAM | Notes |
|-----------|---------|----------|-------|
| LLM (Phi-3 / Llama-3 8B) | 6 GB | 4 GB | `int4` or `int8` quantization |
| STT (faster-whisper base) | 1 GB | 0 GB | CPU-only, `int8` |
| Embeddings (MiniLM) | 300 MB | 0 GB | CPU-only |
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
- Never block the async runtime.
- No circular dependencies.
- Privacy is default.

---

*Last updated: 2026-09-02*