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

## What MAVIS Is Not

- A browser-based chat interface
- A cloud-dependent service
- An autonomous agent that acts without permission
- A memory-hungry process that slows your desktop

## Architecture

┌─────────────┐     ┌─────────────┐
│  Living Orb │────▶│   Context   │
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
└────────────│  AI Worker  │
│  (Python)   │
└─────────────┘


**Runtime split:**
- **`mavis_core` (Rust):** UI, event bus, context engine, memory, system integration. ~50 MB. Always on.
- **`mavis_worker` (Python):** AI inference, model weights, voice. Spawned on demand. Killed when idle.

## Status

| Phase | What | Status |
|-------|------|--------|
| 1 — Foundation | Rust runtime, event bus, orb window | 🚧 In progress |
| 2 — Core Runtime | Context engine, memory, system integration | Not started |
| 3 — AI Worker | Local LLM, Rust–Python bridge | Not started |
| 4 — Integration | Voice pipeline, intent system, automations | Not started |
| 5 — Polish | Performance, packaging, daily driver | Not started |

Full roadmap in [`PHASES.md`](PHASES.md).

## Tech Stack

| Layer | Tools |
|-------|-------|
| Runtime | Rust, tokio, serde, rusqlite |
| UI | winit → raw Wayland |
| System | DBus, inotify, global hotkeys |
| AI | Python, llama.cpp (Q4_K_M) |
| Voice | porcupine, whisper.cpp, piper |
| Audio | cpal |

## Development

```bash
# Rust core
cd mavis_core
cargo run

# Python worker
cd mavis_worker
python -m venv .venv
source .venv/bin/activate
pip install -e .
python -m mavis

## License

MAVIS Source-Available License — see [`LICENSE`](LICENSE).

You are free to use, modify, and share MAVIS. Selling MAVIS as a standalone
product or service is strictly prohibited.