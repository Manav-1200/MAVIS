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

**Protocol:** JSON over UDS (Unix domain socket). No HTTP. No gRPC.

**Platform layer:** Abstracted traits for window tracking, clipboard, screen capture. Linux (Wayland/X11) implemented. Windows and macOS stubbed. TTS engine selection (`MAVIS_TTS_ENGINE=piper|kokoro`) is handled in the executor, not the platform layer.

## Status

| Phase | What | Status |
|-------|------|--------|
| 1 — Foundation | Rust runtime, event bus, orb window | ✅  Complete |
| 2 — Core Runtime | Context engine, memory, system integration | ✅  Complete |
| 3 — AI Worker | Local LLM, Rust–Python bridge | ✅  Complete |
| 4 — Integration | Voice pipeline, intent system, automations | ✅  Complete |
| 5 — Interaction Polish | TTS queue, interruption, session recovery, personality | ✅  Complete |
| 6 — Context Awareness | Active window, clipboard, browser, IDE | Not started |
| 7 — Memory & Learning | Semantic recall, vector embeddings, routines | Not started |
| 8 — Safety & Permissions | Permission tiers, risk scoring, audit log | Not started |
| 9 — Skills Platform | Plugin API, manifest, sandboxing | Not started |
| 10 — Automation & Wellness | Rule engine, proactive suggestions, wellness | Not started |
| 11 — Vision & Advanced UX | OCR, screenshot understanding, dashboard | Not started |
| 12 — Multi-Model & Cross-Platform | Model routing, Windows, macOS | Not started |

Full roadmap in [`PHASES.md`](PHASES.md).

## Tech Stack

| Layer | Tools |
|-------|-------|
| Runtime | Rust, tokio, serde, rusqlite |
| UI | winit -> raw Wayland |
| System | DBus, inotify, global hotkeys |
| AI | Python, llama-cpp-python (Q4_K_M) |
| STT | faster-whisper (CPU int8) |
| TTS | piper, kokoro |
| Audio | cpal |
| Bridge | UDS + length-prefixed JSON |
| Platform | Traits for Linux/Windows/macOS |

## Development

```bash
# Rust core
cd mavis_core
cargo run

# Python worker (in separate terminal, or auto-spawned by bridge)
cd mavis_worker
python -m venv .venv
source .venv/bin/activate
pip install -e .
python -m mavis
```

## Requirements

- Linux (Wayland or X11), Windows, or macOS
- NVIDIA GPU with 6GB+ VRAM recommended (RTX 4050 tested)
- Python 3.10+
- Rust 1.80+

## License

MAVIS Source-Available License — see [`LICENSE`](LICENSE).

You are free to use, modify, and share MAVIS. Selling MAVIS as a standalone
product or service is strictly prohibited.