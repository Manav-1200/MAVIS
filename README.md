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

## What MAVIS Is Not

- A browser-based chat interface
- A cloud-dependent service
- An autonomous agent that acts without permission
- A memory-hungry process that slows your desktop

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

**Protocol:** JSON over UDS (Unix domain socket). No HTTP. No gRPC.

## Status

| Phase | What | Status |
|-------|------|--------|
| 1 — Foundation | Rust runtime, event bus, orb window | ✅ Complete |
| 2 — Core Runtime | Context engine, memory, system integration | ✅ Complete |
| 3 — AI Worker | Local LLM, Rust–Python bridge | ✅ Complete |
| 4 — Integration | Voice pipeline, intent system, automations | 🚧 In Progress |
| 5 — Polish | Performance, packaging, daily driver | Not started |

Full roadmap in [`PHASES.md`](PHASES.md).

## Tech Stack

| Layer | Tools |
|-------|-------|
| Runtime | Rust, tokio, serde, rusqlite |
| UI | winit → raw Wayland |
| System | DBus, inotify, global hotkeys |
| AI | Python, llama-cpp-python (Q4_K_M) |
| STT | faster-whisper (CPU int8) |
| TTS | piper |
| Audio | cpal |
| Bridge | UDS + length-prefixed JSON |

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
python -m mavis.worker
```

## Requirements

- Arch Linux (or any Linux with PipeWire/ALSA)
- NVIDIA GPU with 6GB+ VRAM recommended (RTX 4050 tested)
- Python 3.10+
- Rust 1.80+

## License

MAVIS Source-Available License — see [`LICENSE`](LICENSE).

You are free to use, modify, and share MAVIS. Selling MAVIS as a standalone
product or service is strictly prohibited.
