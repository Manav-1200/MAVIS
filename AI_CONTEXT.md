
---

## `AI_CONTEXT.md`

```markdown
# MAVIS — AI_CONTEXT.md
# Canonical Architecture & Engineering Document
# Version: 0.2.0

## Identity

MAVIS is a persistent desktop-native AI companion. Not a chatbot. Not a web app.
Always present. Never intrusive. Local-first. Privacy-first.

## Runtime Split

- **mavis_core (Rust):** Runtime, UI, event bus, context engine, system glue. Always alive.
- **mavis_worker (Python):** AI inference only. Spawned on demand. Killed when idle.

## Core Principles

1. Voice is interface, not identity.
2. Context Engine is the central nervous system.
3. Planner decides. Executor acts. Never the same module.
4. Memory is layered: Permanent → Long-Term → Episodic → Session → Working.
5. AI-agnostic. Worker is swappable.
6. Event-driven. All subsystems communicate via Event Bus.
7. "Always present. Never intrusive."
8. Rust owns the desktop. Python owns the intelligence.

## Subsystem Boundaries

### Event Bus (Rust)
- Single async pub/sub bus. tokio broadcast.
- Events are small. Large payloads go through bridge.

### Context Engine (Rust)
- Maintains Working Memory.
- Decides promotion/demotion across layers.
- Exposes context window to worker.

### Planner (Rust)
- Receives intent. Decides strategy.
- Never executes. Passes plan to Executor.

### Executor (Rust)
- Receives plan. Performs actions.
- Reports results to Event Bus.

### Memory (Rust)
- Permanent: SQLite. Identity, preferences.
- Long-Term: SQLite. Learned patterns.
- Episodic: SQLite. Events with timestamps.
- Session: SQLite. Conversation thread.
- Working: In-memory. Active context.

### UI — Living Orb (Rust)
- Wayland-native.
- States: Idle, Listening, Thinking, Speaking, Working, Notification, Error, Sleeping.
- Voice-first. No text input by default.

### System Integration (Rust)
- DBus, file watchers, hotkeys, workspace awareness.

### AI Worker (Python)
- Receives context blob + intent.
- Returns response + actions.
- Model weights live here. Never in Rust.

## Data Flow

1. System event → Event Bus
2. Event Bus → Context Engine
3. Context Engine → Planner
4. Planner → Executor
5. Executor → AI Worker
6. AI Worker → Event Bus → Context Engine → UI

## Technology Stack

- Rust: tokio, winit/softbuffer, serde, rusqlite
- Python: transformers, llama-cpp-python, fastapi
- Audio: whisper.cpp, porcupine, piper, cpal
- GPU: CUDA (optional, for local inference acceleration)

## Design Rules

- Core never depends on UI. UI depends on Core.
- Memory never depends on Voice. Voice publishes events.
- Planner never depends on Orb. Orb subscribes to events.
- Python worker never touches filesystem outside its sandbox.
- No circular dependencies.

## Engineering Standards

### Rust
- One module, one responsibility. Max 500 lines.
- All I/O async. CPU-bound work uses `spawn_blocking`.
- Bridge: JSON over stdin/stdout. No pyo3.
- Errors: `anyhow` for app, `thiserror` for libs.
- No `unsafe` without justification.

### Python
- 3.10+. Type hints. Short functions.
- Never `print()`. Always `logging`.
- Never silently ignore exceptions.

### Git
- Conventional commits: `feat:`, `fix:`, `docs:`, `refactor:`, `test:`, `perf:`, `chore:`.
- Small, logical commits.
- `ruff format` and `ruff check` before commit.

### Testing
- Rust: `cargo test`
- Python: `pytest`
- Target: 70% coverage both.

### Performance
- Rust startup < 2s, memory < 200 MB.
- Worker spawn + load < 5s, VRAM < 5 GB.

## Decisions

- **ADR-001:** Single companion identity.
- **ADR-002:** Planner and Executor are separate.
- **ADR-003:** Rust primary runtime, Python for AI only.

## Rule

If a feature is difficult to explain, the design is wrong.