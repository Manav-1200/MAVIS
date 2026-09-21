# MAVIS — AI_CONTEXT.md

Canonical orientation for anyone — human or AI — about to change MAVIS. Read this first, then:

- [`PHASES.md`](PHASES.md) — what exists, phase by phase, and what's verified
- [`DECISIONS.md`](DECISIONS.md) — why each thing is the way it is, and every problem hit so far

*Last updated: 2026-09-21*

---

## Identity

MAVIS is a persistent desktop-native AI companion. Not a chatbot. Not a web app.
**Always present. Never intrusive. Local-first. Privacy-first.**

## Runtime split

- **`mavis_core` (Rust):** runtime, orb, event bus, context engine, memory, safety, system integration. Always alive. ~50 MB.
- **`mavis_worker` (Python):** LLM, STT and TTS inference only. Spawned on demand, unloads models when idle.

## Architecture

```
STT (cpal + VAD) ──UserIntent──▶ EventBus ◀── every subsystem publishes and subscribes
                                    │
   ContextEngine ◀──────────────────┤  owns working memory; records to recall/episodic
   Planner ◀────────────────────────┤  deterministic intents first, LLM for the rest
      │ PlanReady                   │
   PermissionGate ◀─────────────────┤  scores risk, writes audit.db, asks if needed
      │ PlanApproved                │
   Executor ◀───────────────────────┤  the only thing that acts
   Sentinel ──SystemChange──────────┤  watches what changed on the machine
   Orb ◀────────────────────────────┘  UiStateChange drives its state

Planner ◀──UDS, length-prefixed JSON──▶ mavis_worker (Python)
```

- The **event bus** is in-process `tokio::sync::broadcast`. Only the Python worker is reached over a socket (`/tmp/mavis_worker.sock`).
- The **executor listens for `PlanApproved`, never `PlanReady`.** Nothing reaches it without passing the permission gate.
- Every event-loop subsystem runs under **`util::supervise`**: a panic is logged and the subsystem rebuilt and restarted, up to 5 times.

## Memory

| Tier | Store | Lifetime |
|---|---|---|
| Working | in-RAM, `working_memory.json` | session; conversational events only |
| Recall | `recall.db` (FTS5) | 7 / 30 / 90 days / forever, by importance |
| Long-term | `long_term.db` (FTS5) | one summary per day, permanent |
| Episodic | `episodic.db` | written, not yet read |
| Entities | `entities.db` | projects, apps, files and their co-occurrence |
| Audit | `audit.db` | append-only |
| Sentinel | `sentinel.db` | watermarks and change history |

All in `../memory`, relative to `mavis_core/`. *(Permanent and Session stores were planned and removed — nothing read them.)*

---

## Non-negotiable rules

1. **The LLM never makes a consequential decision.** It does not select actions, score risk, rate memory importance or decide severity. It may phrase things. It may *raise* a risk score as a second opinion; it may never lower one. The model is a 3.8B Phi-3 that is unreliable at format rules.
2. **Nothing runs without the permission gate.** Don't add a path from the planner, or anything else, straight to the executor.
3. **`shell` stays unreachable from voice.** The gate scores shell commands; the planner still never produces them.
4. **Context sources are opt-in.** Each has a `MAVIS_CONTEXT_*` variable, default off. So does the Sentinel (`MAVIS_SENTINEL`).
5. **Audit and change logs are append-only.** No update or delete methods on `AuditLog`; the Sentinel only updates its `announced` flag.
6. **MAVIS reports facts, not malware verdicts.** Real scanners' findings are reported as theirs.
7. **Don't slice strings by byte index.** Use `util::truncate_bytes` or `str::get`. All input — transcriptions, clipboard, window titles — is external and may be non-ASCII. This bug class has shipped twice.
8. **Don't block the async runtime.** CPU-bound or blocking work goes to `spawn_blocking` or its own thread.
9. **Add a dependency only when it has earned its place.** FTS5 over embeddings; heuristics over spaCy.
10. **"Always present. Never intrusive."** Every feature has to pass it.

## Looks wrong, but is deliberate

Each of these has been "improved" before, or nearly was. The reasons are in `DECISIONS.md`.

| What | Why |
|---|---|
| VAD start 0.075 / end 0.06, silence 500 ms | Measured on the target mic. The longest dip inside real speech is 450 ms — lower splits sentences. If a mic seems deaf, raise its volume; don't lower thresholds. |
| First 1.5 s of audio discarded; utterances under 1.5 s dropped | Opening the stream emits a full-scale click that Whisper turns into invented sentences. |
| Recall returns only `role = 'user'` | Returning MAVIS's own replies made it treat its guesses as remembered facts. |
| `ContextUpdate` and `SystemChange` kept out of the working-memory ring | Polling every 2 s evicted the conversation in about two minutes. |
| Intents matched by phrase, not by the LLM | Faster, and an LLM choosing actions makes its mistakes consequential. |
| Explicit prefixes (`google `, `open `, `play `…) block system intents | "google how to mute a tab" used to mute the machine. |
| `worker.py` uses `print(..., flush=True)` | The worker's stdout is piped and block-buffered; without the flush, logs arrived seconds late and out of order. |
| Compositor detection *runs* each candidate instead of trusting env vars | A leaked `NIRI_SOCKET` made a GNOME session think it was niri. |
| Sentinel's first run announces nothing | A real machine has thousands of logged transactions. |
| Sentinel filters `>=` its watermark, not `>` | A scan landing mid-transaction lost lines sharing its timestamp. The fingerprint deduplicates. |
| Sentinel speaks one sentence per update | On Arch ~90% of packages are dependencies; per-package would be unbearable. |
| `MAVIS_ORB=off` exists but isn't for testing | The orb is the only live signal of what MAVIS is doing. |
| No vector embeddings, no spaCy | Three dependencies and an 80 MB model for little gain; spaCy misses exactly the entities that matter here. |

## Engineering standards

**Rust**
- `anyhow` for application errors, `thiserror` for library errors. No `unsafe` without justification.
- No `.unwrap()` / `.expect()` on anything reachable at runtime. Recover mutex poison rather than propagating it.
- Every module gets `cargo test` before it's done. Pure logic — parsers, scoring, matching — is kept pure so it can be tested without hardware.
- Guideline, not yet met: one responsibility per module, around 500 lines. `planner.rs`, `main.rs`, `platform/linux.rs` and `sentinel/packages.rs` exceed it.

**Python**
- 3.10+, type hints, short functions. Never silently swallow exceptions.
- `logging` for library code; the worker's lifecycle output uses `print(..., flush=True)` deliberately — see above.

**Changes**
- Conventional commits: `feat:`, `fix:`, `docs:`, `refactor:`, `test:`, `perf:`, `chore:`. Small and logical.
- **When you fix a bug, search for the same pattern elsewhere.** Fix the class, not the instance.
- **A regression test isn't proven until it has been seen failing** against the old code.
- **Verify before trusting a success message** — on real hardware where it matters.
- Record the decision in `DECISIONS.md` in the same change.

**Build**
- Rust 1.85+ via rustup. Build needs a C compiler, `pkg-config` and ALSA headers — not X11 or Wayland headers.
- `cargo clippy` currently fails on one known error in `executor.rs`.
- `Cargo.lock` is currently gitignored, so builds aren't reproducible across machines. Committing it is recommended.

**Targets (not yet measured)** — Rust startup under 2 s and under 200 MB; worker spawn and model load under 5 s, under 5 GB VRAM; 70% test coverage.

## Architecture decisions

The full records, with context and rejected alternatives, are in [`DECISIONS.md` §3](DECISIONS.md#3-architecture).

| ADR | Decision |
|---|---|
| 001 | Single companion identity |
| 002 | The planner decides; the executor acts |
| 003 | Rust owns the runtime; Python owns the AI |
| 004 | Keep the event bus |
| 005 | UDS with length-prefixed JSON for the worker |
| 006 | Deterministic intent matching before the LLM |
| 007 | Static risk scoring; the LLM may only raise a score |
| 008 | Every context source is opt-in |
| 009 | Supervise every event-loop subsystem |
| 010 | The Sentinel reports facts, never malware verdicts |

## Rule

If a feature is difficult to explain, the design is wrong.