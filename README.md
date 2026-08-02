# MAVIS

> **Modular Autonomous Virtual Intelligence System**

A modular, context-aware, local-first desktop AI companion built with Python and Rust.

---

# Overview

MAVIS is an open-source desktop AI companion designed to integrate naturally with the operating system instead of existing as just another chatbot.

Rather than simply answering prompts, MAVIS is being built to understand context, remember useful information, assist with planning, automate repetitive workflows, and provide intelligent desktop assistance while keeping the user in control.

The project follows a modular architecture where each subsystem has a single responsibility and communicates through well-defined interfaces.

---

# Project Status

Current Version

```
v0.1.0
```

Current Phase

```
Foundation
```

Status

```
Active Development
```

The project is currently focused on building a robust engineering foundation before implementing advanced AI capabilities.

---

# Vision

MAVIS aims to become a reliable desktop AI companion capable of:

- Understanding user context
- Maintaining long-term memory
- Planning multi-step tasks
- Executing workflows safely
- Integrating deeply with the operating system
- Extending functionality through plugins
- Operating locally whenever possible

The goal is to create software that remains understandable, maintainable, and extensible for many years.

---

# Core Principles

- Desktop AI companion first
- Local-first architecture
- Privacy by default
- Modular design
- Event-driven communication
- Layered memory
- Safe automation
- Human-centered interaction
- Cross-platform compatibility

---

# Planned Features

## Core

- Bootstrap system
- Logging
- Configuration management
- Runtime lifecycle
- Event bus

## Memory

- Working Memory
- Session Memory
- Episodic Memory
- Long-Term Memory
- Permanent Memory

## Intelligence

- Context Engine
- Planner
- Executor
- AI provider abstraction

## User Interface

- Living Orb
- Expandable workspace
- Companion display mode

## Voice

- Wake word
- Speech-to-text
- Text-to-speech
- Voice activity detection

## Plugins

- Dynamic plugin loading
- Skill registration
- Plugin permissions
- Public plugin API

## Automation

- Reminders
- Workflow automation
- Desktop integration
- Scheduled tasks

---

# Project Structure

```
MAVIS/

config/
data/
logs/
memory/
plugins/

src/
└── mavis/

tests/

README.md
ROADMAP.md
ARCHITECTURE.md
ENGINEERING.md
MISSION.md
VISION.md
AI_CONTEXT.md
CHANGELOG.md
```

---

# Technology Stack

Primary Language

- Python 3.14+

Performance Components

- Rust

Package Manager

- uv

Formatter

- Ruff

Git Hooks

- pre-commit

Version Control

- Git

---

# Development Setup

Clone the repository

```bash
git clone <repository>
cd MAVIS
```

Install dependencies

```bash
uv sync
```

Run MAVIS

```bash
python -m mavis
```

or

```bash
mavis
```

---

# Development Workflow

Every new feature should follow this process:

1. Update documentation.
2. Design the architecture.
3. Implement the feature.
4. Test the implementation.
5. Commit using a conventional commit message.

Documentation is treated as part of the codebase rather than an afterthought.

---

# Repository Documentation

| Document | Purpose |
|-----------|---------|
| README.md | Project overview |
| ROADMAP.md | Development roadmap |
| ARCHITECTURE.md | Software architecture |
| ENGINEERING.md | Engineering standards |
| MISSION.md | Project mission |
| VISION.md | Long-term vision |
| AI_CONTEXT.md | AI coding guidance |
| CHANGELOG.md | Release history |

---

# Current Progress

Completed

- Project initialization
- Git repository
- uv project
- Virtual environment
- Ruff
- Pre-commit hooks
- Logging
- Bootstrap
- Lifecycle
- Runtime directories
- Documentation foundation

In Progress

- Core architecture
- Configuration system
- Event system

Planned

- Memory engine
- Context engine
- Planner
- Executor
- Orb UI
- Plugin system
- Voice
- Native OS integration

---

# Contributing

Contributions should follow the project's engineering standards.

Before implementing significant changes:

- Read ENGINEERING.md
- Read ARCHITECTURE.md
- Review ROADMAP.md

Maintain consistency with the existing architecture and coding style.

---

# License

License information will be added before the first public release.

---

# Acknowledgements

MAVIS is a long-term engineering project focused on building a trustworthy desktop AI companion through clean architecture, strong engineering practices, and thoughtful design.

---

*"Build software that remains understandable years from now."*