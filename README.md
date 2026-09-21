# MAVIS
**Modular Autonomous Virtual Intelligence System**

A persistent desktop-native AI companion. Not a chatbot. Not a web app.

**Always present. Never intrusive.**

## What MAVIS Is

- A **local-first, privacy-first** desktop companion that stays alive forever
- A **Rust runtime** (~50 MB) that owns the desktop: the orb, the event bus, the context engine
- A **Python AI worker** that loads only when needed, runs your local LLM, then unloads to reclaim VRAM
- A **living orb** that communicates state through subtle animation
- A system that **remembers**, **plans**, and **assists** without taking control
- **Voice-enabled**: speak naturally, MAVIS listens, thinks, and speaks back
- **Cross-platform**: Linux (primary), Windows, macOS — same runtime, different backends

## What MAVIS Is Not

- A browser-based chat interface
- A cloud-dependent service
- An autonomous agent that acts without permission
- A memory-hungry process that slows your desktop
- A chatbot in a window

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

**Runtime split:**
- **`mavis_core` (Rust):** UI, event bus, context engine, memory, safety, system integration. ~50 MB. Always on.
- **`mavis_worker` (Python):** AI inference, model weights, voice. Spawned on demand. Killed when idle.

**Protocol:** subsystems talk over an in-process event bus. The Python worker is reached over a Unix domain socket with length-prefixed JSON. No HTTP. No gRPC.

**Nothing runs unreviewed.** Every plan passes a permission gate that scores its risk and records it in an append-only audit log before the executor sees it.

**Nothing dies silently.** Each subsystem runs under a supervisor. If one panics, it's logged loudly and restarted, rather than quietly disappearing while the rest of MAVIS carries on.

**Platform layer:** Abstracted traits for window tracking, clipboard, screen capture, and project detection. Linux (Wayland/X11) implemented. Windows and macOS stubbed. TTS engine selection (`MAVIS_TTS_ENGINE=piper|kokoro`) is handled in the executor, not the platform layer.

## Status

| Phase | What | Status |
|-------|------|--------|
| 1 — Foundation | Rust runtime, event bus, orb window | :white_check_mark: Complete |
| 2 — Core Runtime | Context engine, memory, system integration | :white_check_mark: Complete |
| 3 — AI Worker | Local LLM, Rust–Python bridge | :white_check_mark: Complete |
| 4 — Integration | Voice pipeline, intent system, automations | :white_check_mark: Complete |
| 5 — Interaction Polish | TTS queue, interruption, session recovery, personality | :white_check_mark: Complete |
| 6 — Context Awareness | Active window, workspace, clipboard, IDE, terminal, project, calendar | :white_check_mark: Complete |
| 6.5 — Action Execution | App launching, search, system control | :white_check_mark: Built |
| 7 — Memory & Learning | Recall with decay, daily consolidation, replay, entity graph | :white_check_mark: 7.1–7.2 complete |
| 8 — Safety & Permissions | Risk scoring, permission gate, confirmation, audit log | :white_check_mark: Core built |
| 8.5 — System Sentinel | Notices packages you didn't ask for | :construction: Detects and records; not yet speaking |
| 9 — Skills Platform | Plugin API, manifest, sandboxing | Not started |
| 10 — Automation & Wellness | Rule engine, proactive suggestions, wellness | Not started |
| 11 — Vision & Advanced UX | OCR, screenshot understanding, dashboard | Not started |
| 12 — Multi-Model & Cross-Platform | Model routing, Windows, macOS | Not started |

Full roadmap in [`PHASES.md`](PHASES.md). **Why** each thing is built the way it is — and every problem hit along the way — is in [`DECISIONS.md`](DECISIONS.md).

## Tech Stack

| Layer | Tools |
|-------|-------|
| Runtime | Rust, tokio, serde, rusqlite |
| UI | minifb |
| System | DBus, inotify, global hotkeys |
| AI | Python, llama-cpp-python (Q4_K_M) |
| STT | faster-whisper `small` (CPU int8) |
| TTS | piper, kokoro |
| Audio | cpal |
| Memory search | SQLite FTS5 |
| Bridge | UDS + length-prefixed JSON |
| Platform | Traits for Linux/Windows/macOS |

## Development

```bash
# Rust core — run from inside mavis_core/ (memory lives at ../memory, relative to here)
cd mavis_core
cargo run

# Python worker (in separate terminal, or auto-spawned by bridge)
cd mavis_worker
python -m venv .venv
source .venv/bin/activate
pip install -e .
python -m mavis
```

`cargo test` runs the unit tests. `cargo clippy` currently reports one known error in `executor.rs` — see [`DECISIONS.md` §14](DECISIONS.md#14-open-issues).

### Voice commands

Some phrases are handled directly by the planner without an LLM round trip, so they respond near-instantly:

| Say | MAVIS does |
|-----|------------|
| "open firefox", "launch terminal" | Launches the app |
| "play lofi hip hop" | Opens a YouTube search |
| "google rust async traits" | Opens a web search |
| "volume up", "louder", "mute" | Adjusts volume |
| "pause", "next song" | Media control |
| "brighter", "dim the screen" | Adjusts brightness |

Applications are discovered from what's actually installed, so this tracks your setup rather than a fixed list. Anything else goes to the local model with context attached.

An utterance that starts with an explicit action is never treated as system control — "google how to mute a tab" searches rather than muting your machine.

`shell` execution exists in the executor and is scored by the permission gate, but it is **deliberately unreachable from voice**.

### Permissions

Every plan is scored for risk before anything runs:

| Risk | What happens |
|------|--------------|
| 0–2 | Runs silently — speaking, notifications, volume, launching an app |
| 3–7 | "Shall I?" — waits up to 20 seconds for a clear yes |
| 8+ | Requires you to say "yes, administrator" |
| Irreversible | Refused, whatever you say |

Anything that isn't a clear yes cancels. A "no" anywhere in the answer always wins. Every decision is written to `memory/audit.db`, which MAVIS can add to but never edit.

### Memory

MAVIS remembers across sessions. Five tiers, all local SQLite:

| Tier | Contents | Lifetime |
|------|----------|----------|
| Working | current session, context snapshot | in-RAM + JSON |
| Episodic | raw event log | indefinite |
| Recall | what you said, importance-scored | 7–90 days by importance; stated facts kept |
| Long-term | one summary per day | permanent |
| Entities | projects, apps, files and what co-occurs | permanent |

Search uses SQLite FTS5 rather than vector embeddings — no extra dependency, no model download. Entities come from observed facts (git repo names, app IDs, filenames), not inference. MAVIS recalls what *you* said, never its own earlier replies — otherwise one guess becomes a remembered "fact".

### System Sentinel

With `MAVIS_SENTINEL=1`, MAVIS reads your package manager's own log and notices what changed — especially packages that arrived as a dependency of something else, which is how you end up with software you never chose.

- Works with **pacman**, **dpkg/apt** and **dnf** (dnf untested on real hardware)
- The first run imports your history silently; only changes after that are reported
- One sentence per update, not one per package
- Ordinary upgrades are never mentioned
- It reports facts. It does not judge whether anything is malware.

It currently detects and records. Speaking about changes and answering "what changed recently?" are next — see [`PHASES.md` Phase 8.5](PHASES.md#phase-85--system-sentinel).

### Context sources

Every context source is opt-in and off by default. Enable the ones you want:

```bash
MAVIS_CONTEXT_ACTIVE_WINDOW=1 \
MAVIS_CONTEXT_CLIPBOARD=1 \
MAVIS_CONTEXT_CALENDAR=1 \
cargo run
```

| Variable | Gives MAVIS |
|----------|-------------|
| `MAVIS_CONTEXT_ACTIVE_WINDOW` | Focused window, the list of open apps and their workspaces, and the current git project (name, path, branch) |
| `MAVIS_CONTEXT_CLIPBOARD` | Current clipboard text, truncated to 200 characters |
| `MAVIS_CONTEXT_CALENDAR` | Next event from Evolution's local calendar |
| `MAVIS_CONTEXT_BROWSER` | Tab URL/title, if something writes to `/tmp/mavis_browser.sock` |

Current date and time is always injected — it isn't private, and without it the model guesses at anything time-related.

### All environment variables

| Variable | Default | Effect |
|----------|---------|--------|
| `MAVIS_CONTEXT_*` | off | The four context sources above |
| `MAVIS_SENTINEL` | off | Package change detection |
| `MAVIS_TTS_ENGINE` | `piper` | `piper` or `kokoro` |
| `MAVIS_VOICE_MODEL` | `~/.local/share/piper-voices/en_US-lessac-medium.onnx` | Piper voice model |
| `MAVIS_KOKORO_VOICE` | `af_heart` | Kokoro voice |
| `MAVIS_AUDIO_DEVICE` | auto | Exact microphone name as cpal reports it |
| `MAVIS_PYTHON_PATH` | `python3` | Interpreter for the worker, e.g. your venv's |
| `MAVIS_ORB` | on | `off` runs without the orb — headless use only; the orb is how you see what MAVIS is doing |
| `MAVIS_ORB_POS` | — | `x,y` start position. Honoured on X11/XWayland; native Wayland ignores it |
| `MAVIS_VAD_DEBUG` | off | Logs what the microphone is actually producing, every ~3 s |

## Troubleshooting

**MAVIS doesn't hear you.** Check the microphone level before anything else — it has cost a full debugging session before:

```bash
wpctl status | grep -A3 Sources
wpctl set-volume @DEFAULT_AUDIO_SOURCE@ 0.6
```

Then run with `MAVIS_VAD_DEBUG=1` and speak. If `peak=` stays below ~0.075 while you talk, the microphone is too quiet — raise it rather than lowering MAVIS's thresholds, which are calibrated against measured room noise.

**A subsystem stops working mid-session.** Look for `PANICKED — restarting` in the log. The panic message just above it is the bug worth reporting.

**The orb won't move on Wayland.** Native Wayland doesn't let applications position their own windows. Use XWayland, or drag it.

## Requirements

- Linux (Wayland or X11), Windows, or macOS
- NVIDIA GPU with 6GB+ VRAM recommended (RTX 4050 tested)
- Python 3.10+
- **Rust 1.85+**. [rustup](https://rustup.rs) is the simplest route everywhere. Default distribution packages are often too old: Debian 12 ships 1.63, and Ubuntu 24.04's default `rustc` is 1.75 — though Ubuntu also offers versioned packages (`sudo apt install rustc-1.85 cargo-1.85`).
- Build dependencies — a C compiler, `pkg-config` and the ALSA headers:
  - Debian/Ubuntu: `sudo apt install build-essential pkg-config libasound2-dev`
  - Arch: `sudo pacman -S --needed base-devel alsa-lib`
  - Fedora: `sudo dnf install gcc pkg-config alsa-lib-devel`

  X11 and Wayland headers are *not* needed: the orb loads those libraries at runtime, and any desktop already has them.

## License

MAVIS Source-Available License — see [`LICENSE`](LICENSE).

You are free to use, modify, and share MAVIS. Selling MAVIS as a standalone
product or service is strictly prohibited.