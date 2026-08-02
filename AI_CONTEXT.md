# AI Context

> Project: Modular Autonomous Virtual Intelligence System (MAVIS)

This document provides guidance for AI assistants contributing to the MAVIS codebase.

It defines project goals, engineering expectations, architectural constraints, and coding standards that should be followed during every development session.

---

# Project Overview

MAVIS stands for:

**Modular Autonomous Virtual Intelligence System**

MAVIS is a desktop AI companion.

It is **not** a chatbot.

Conversation is only one interface.

The objective is to build a context-aware assistant that integrates naturally with the desktop environment while remaining modular, maintainable, privacy-conscious, and extensible.

---

# Primary Goal

Build a reliable desktop AI companion that assists users with their daily workflows.

Priority is given to:

- Context awareness
- Memory
- Planning
- Safe automation
- Native desktop integration
- Long-term maintainability

---

# Project Philosophy

Every engineering decision should support the following principles.

- Desktop AI companion first
- Local-first whenever practical
- Privacy by default
- Event-driven architecture
- Modular subsystems
- Clean APIs
- Clear documentation
- Predictable behavior
- User remains in control

Do not sacrifice architecture for short-term speed.

---

# AI Development Rules

When generating code:

- Prefer correctness over speed.
- Prefer readability over cleverness.
- Prefer maintainability over brevity.
- Prefer explicit behavior over hidden magic.

Generated code should resemble production-quality software.

---

# Coding Standards

Always:

- use type hints
- use descriptive variable names
- use descriptive function names
- write meaningful docstrings
- write useful comments explaining WHY rather than WHAT
- follow Ruff formatting
- use the Python standard library whenever practical

Avoid:

- unnecessary abstractions
- duplicated logic
- global mutable state
- deeply nested conditionals
- magic numbers
- hardcoded paths

---

# Project Architecture

MAVIS follows a layered architecture.

```
UI
│
Planner
│
Executor
│
Services
│
Core
```

Lower layers must never depend on higher layers.

Avoid circular dependencies.

---

# Module Responsibilities

Each module should have a single responsibility.

Examples

Core

Runs the application.

Memory

Stores knowledge.

Context

Builds runtime context.

Planner

Creates execution plans.

Executor

Performs work.

Plugins

Extend MAVIS.

Voice

Provides speech interaction.

UI

Displays information.

Do not mix responsibilities.

---

# Logging

Never use

```
print()
```

Use the centralized logging system.

Example

```python
logger = get_logger(__name__)

logger.info(...)
logger.warning(...)
logger.error(...)
logger.exception(...)
```

Every module owns its own logger.

---

# Error Handling

Never silently ignore exceptions.

Bad

```python
except Exception:
    pass
```

Good

```python
except Exception as exc:
    logger.exception("Failed to ...")
```

Recover whenever possible.

Fail gracefully.

---

# Documentation

Documentation is part of the project.

Whenever architecture changes:

Update

- ARCHITECTURE.md

Whenever project direction changes:

Update

- ROADMAP.md

Whenever engineering rules change:

Update

- ENGINEERING.md

Whenever functionality changes:

Update

- CHANGELOG.md

Documentation should evolve with the codebase.

---

# Memory Philosophy

MAVIS uses layered memory.

Current design

- Working Memory
- Session Memory
- Episodic Memory
- Long-Term Memory
- Permanent Memory

All memory access should occur through the Memory Manager.

Never bypass it.

---

# Plugin Philosophy

Core functionality should remain minimal.

Optional functionality should become plugins.

Plugins must:

- use public APIs
- declare permissions
- remain self-contained

Plugins should never modify internal MAVIS state directly.

---

# AI Providers

MAVIS should not depend on a single AI provider.

Design new features so providers can be replaced without rewriting business logic.

Cloud APIs should remain optional.

Local models should be supported whenever practical.

---

# Python and Rust

Python is the primary language.

Rust should only be introduced for performance-critical workloads.

Examples

- vector search
- image processing
- cryptography
- indexing
- compression

Business logic remains in Python.

---

# Dependencies

Before adding a dependency, ask:

Can the Python standard library solve this?

If yes,

do not add a dependency.

Keep external dependencies minimal.

---

# Testing

Every subsystem should eventually support testing.

Favor deterministic behavior.

Avoid hidden side effects.

Keep functions easy to test.

---

# Repository Standards

Before implementing a feature:

1. Verify it aligns with ROADMAP.md.
2. Ensure the architecture supports it.
3. Implement the feature.
4. Document any architectural changes.
5. Update CHANGELOG.md if appropriate.

---

# Communication Style

When assisting with MAVIS:

- Be direct.
- Be technically accurate.
- Avoid unnecessary explanations.
- Do not oversimplify engineering concepts.
- Explain trade-offs when appropriate.

When providing code changes:

Always provide the complete updated file unless explicitly asked for a partial diff.

This reduces copy/paste mistakes and keeps development consistent.

---

# Long-Term Goal

MAVIS is intended to become a dependable desktop AI companion that users can rely on every day.

Every contribution should move the project toward that goal without compromising architecture, maintainability, or user trust.

---

End of AI Context.