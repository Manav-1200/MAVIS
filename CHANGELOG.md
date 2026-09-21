# Changelog

What changed, by release. For **why**, see [`DECISIONS.md`](DECISIONS.md); for the full roadmap and verification status, [`PHASES.md`](PHASES.md).

Format loosely follows [Keep a Changelog](https://keepachangelog.com/).

> **Versioning note:** only `v0.3.0-ai-worker` was ever tagged. The `v0.1.0` and `v0.2.0` entries below mark phases, not tags. The two halves also disagree — `mavis_core` says `0.1.0`, `mavis_worker` says `0.3.0`. Everything since the tag is listed under Unreleased.

---

## [Unreleased]

192 commits since `v0.3.0-ai-worker`, covering Phases 4 through 8.5.

### Added
- **System Sentinel** (Phase 8.5, opt-in via `MAVIS_SENTINEL=1`) — reads the pacman, dpkg or dnf transaction log and notices packages you didn't ask for, removals and downgrades. First run imports history silently; one sentence per update; routine upgrades never mentioned. Detects and records; speaking is not yet wired up.
- **Permission gate** (Phase 8) — every plan is scored 0–10 and recorded before the executor sees it. Silent below 3, "Shall I?" from 3, "yes, administrator" from 8, irreversible commands refused.
- **Append-only audit log** — `audit.db`, with no update or delete path.
- **Subsystem supervision** — a panicking subsystem is logged and restarted instead of silently disappearing.
- **Memory** (Phase 7) — FTS5 recall with importance-based decay, hourly daily-summary consolidation, time-range replay ("what was I doing yesterday afternoon"), and an entity graph of projects, apps and files.
- **Actions** (Phase 6.5) — launch any installed app, YouTube and web search, URLs, and voice control of volume, media and brightness. Apps discovered from `.desktop` files, Start Menu shortcuts or `.app` bundles.
- **Context awareness** (Phase 6) — active window, open windows and workspaces, clipboard, IDE and terminal awareness, git project and branch, calendar, date and time. Each source opt-in via `MAVIS_CONTEXT_*`.
- **Voice** (Phases 4–5) — faster-whisper STT, Piper and Kokoro TTS, a non-blocking TTS queue with interruption, session recovery across restarts, and the celebrating orb state.
- `MAVIS_VAD_DEBUG` to see what the microphone is actually producing.
- `MAVIS_ORB=off` and `MAVIS_ORB_POS=x,y`.

### Fixed
- **MAVIS stopping mid-session** — three places sliced strings by byte index and panicked on non-ASCII transcriptions or clipboard text; the panic silently ended that subsystem. Most likely cause of the intermittent "doesn't respond" reports.
- **Event bus poison cascade** — one panic while holding its lock made every later publish panic.
- **Audio-thread deadlock** after any mutex poisoning.
- **GNOME session crash** — project detection rescanned `/proc` tens of thousands of times every 2 seconds.
- **Compositor misdetection** from leaked environment variables; failed commands being read as success.
- **Voice activity detection** — silence was undetectable because the threshold sat below the noise floor; rebuilt from measured traces with start/end hysteresis.
- **Invented transcriptions** — from the audio stream's startup click, and from Whisper hallucinating on near-silence.
- **MAVIS hearing its own voice** — echo suppression checked a flag that nothing ever set.
- **MAVIS treating its own guesses as memories** — recall now returns only what the user said.
- **Conversation evicted by polling** — context updates no longer enter the working-memory ring.
- **"google how to mute a tab" muting the machine.**
- Missing microphone no longer aborts the whole process.
- App discovery now searches XDG data directories.
- Deny-list falsely refusing `rm -rf` on a project directory; `curl … | bash` slipping through.
- Audit log recording held actions as "blocked".
- Sentinel losing log entries written in the same second as its watermark.

### Removed
- `PermanentStore` and `SessionStore` — Phase 1 stubs nothing read.
- Duplicate Python VAD and microphone code; dead scaffold modules.
- Browser-extension approach to tab awareness (the receiving socket is kept).

### Known issues
See [`DECISIONS.md` §14](DECISIONS.md#14-open-issues). Most notably: `cargo clippy` fails on one error in the executor's espeak fallback, which can never run; the worker socket is world-writable; `app` actions bypass shell risk scoring; and `Cargo.lock` is not committed.

---

## [0.3.0] — 2026-08-12 · `v0.3.0-ai-worker`

### Added
- Rust ↔ Python bridge over a Unix domain socket with length-prefixed JSON.
- Local LLM via llama-cpp-python (Phi-3-mini, Q4_K_M) with CUDA offload.
- Worker lifecycle: eager spawn, health checks, idle model unload, crash recovery with auto-respawn.
- Prompt system with a defined MAVIS personality and working-memory injection.
- `MAVIS_PYTHON_PATH` for the worker's interpreter.

## [0.2.0] — 2026-08-06 · *Phase 2, not tagged*

### Added
- Context engine routing every event type, with working memory.
- SQLite-backed memory layers.
- Planner and executor, with `shell`, `app`, `notify`, `say` and `system` actions.
- System integration: DBus media, volume and brightness; hotkey socket; file watcher.
- Orb state machine.

## [0.1.0] — 2026-08-04 · *Phase 1, not tagged*

### Added
- Project scaffold with Rust runtime and Python worker.
- Async event bus (tokio broadcast).
- Subsystem stubs for all Phase 1 modules.
