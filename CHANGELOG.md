# Changelog

What changed, by release. For **why**, see [`DECISIONS.md`](DECISIONS.md); for the full roadmap and verification status, [`PHASES.md`](PHASES.md).

Format loosely follows [Keep a Changelog](https://keepachangelog.com/).

> **Versioning note:** only `v0.3.0-ai-worker` was ever tagged. The `v0.1.0` and `v0.2.0` entries below mark phases, not tags. The two halves also disagree — `mavis_core` says `0.1.0`, `mavis_worker` says `0.3.0`. Everything since the tag is listed under Unreleased.

---

## [Unreleased]

192 commits since `v0.3.0-ai-worker`, covering Phases 4 through 8.5.

### Added
- **Barge-in** — talk over MAVIS and it stops speaking immediately; say "stop" and it goes quiet without answering. It listens while it talks, against a threshold set above its own voice. `MAVIS_BARGE_IN=0` turns it off.
- **"Sorry, I didn't catch that"** when a transcript is too unreliable to act on, instead of answering a misheard question.
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
- **MAVIS said "Opening Firefox" without opening anything** — commands were only recognised at the start of a sentence, a comma after the wake word blocked them, and an app didn't match unless every spoken word was in its name. The model is also now told it cannot act, so it stops narrating actions.
- **Repeating its own last reply** on short or garbled input — less history in the prompt, and known facts now sit last where they outrank the conversation.
- **Real speech dropped** as "too short" (minimum 1.5 s → 0.8 s) or as room noise; the noise floor no longer drifts upward on word onsets and echo.
- **Questions stored as memories**, which fed the user's old questions back into every prompt.
- **The worker's own logs never appeared** — logging was configured but never switched on.
- Kokoro contacted Hugging Face on every start even with the model cached.
- **60-second replies** (found in the 2026-09-22 live run) — three causes, all fixed:
  - *Listening never ended in a noisy room.* The VAD's thresholds couldn't rise above fixed values, so fan noise held utterances open to the 45 s ceiling. The noise floor is now measured at startup, follows the room, survives between utterances, and is re-measured mid-utterance if the room gets louder; trailing noise is trimmed. Ceiling 45 s → 30 s.
  - *Whisper invented "Thank you for watching, please subscribe…"* after real speech. Silero VAD, already bundled with faster-whisper, now marks which audio is speech; Whisper hears only that and isn't called at all without it. The hallucination phrase lists are gone.
  - *The LLM generated up to 256 tokens and kept ~40.* Generation now streams and stops once the reply is complete; the system prompt is pre-evaluated while the user is still speaking.
- **Unpunctuated transcripts came back empty** — sentence de-duplication dropped text after the last punctuation mark.
- **"The user's name is Using"** — names are recognised by how they're said (explicit introduction, last in the clause, not a question), with no word lists; names stored by older builds are discarded once.
- **MAVIS saw its own orb as the active window**, and reported its own directory as the user's project.
- **Every reply was in the prompt twice**, and the current utterance could be too.
- **Clipboard sent with every message** — now only when asked about.
- **Speech fallback never fell back** from `spd-say` to `espeak`, and `spd-say` didn't wait for speech to finish. `cargo clippy` passes again.
- **Integer-format microphones** (16/32-bit) were rejected as unsupported.
- Context logged every 2 s, including clipboard text — now logged on change, as a length.
- Kokoro's `repo_id` and torch warnings on every load.
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