# MAVIS Architecture

> Modular Autonomous Virtual Intelligence System

Version: 0.1.0

---

# Purpose

This document describes the complete software architecture of MAVIS.

It serves as the primary engineering specification for the project and should always reflect the current design of the system.

Whenever the architecture changes, this document must be updated before or alongside the implementation.

---

# Design Philosophy

MAVIS is **not** designed as a chatbot.

MAVIS is a **desktop AI companion**.

The AI model is only one component inside a much larger operating framework.

The architecture emphasizes:

- Modularity
- Extensibility
- Reliability
- Maintainability
- Local-first execution
- Event-driven communication
- Separation of concerns
- Minimal dependencies
- Cross-platform compatibility

Every subsystem should have one clear responsibility.

---

# High-Level Architecture

```

```
                   User
                     │
                     ▼
            Orb / Voice / UI
                     │
                     ▼
             Context Engine
                     │
                     ▼
              Planner Engine
                     │
                     ▼
              Executor Engine
                     │
        ┌────────────┼────────────┐
        ▼            ▼            ▼
    Plugins       Skills       Services
        │            │            │
        └────────────┼────────────┘
                     ▼
               Event Bus
                     ▼
            Memory Manager
                     ▼
      Working / Session / Episodic
      Long-Term / Permanent
```

---

# Core Principles

## Modular Design

Every subsystem exists independently.

Modules communicate through interfaces rather than direct dependencies whenever practical.

Benefits

- Easier testing
- Easier maintenance
- Easier replacement
- Easier extension

---

## Event Driven

Subsystems should communicate through events whenever possible.

Example

```

Plugin Installed

↓

Event Published

↓

Interested Modules Receive Notification

↓

React Independently

```

This avoids tightly coupling unrelated components.

---

## Local First

MAVIS should always prefer local resources.

Priority

1. Local processing
2. Local models
3. Local storage
4. Cloud only when requested or required

User privacy takes priority over convenience.

---

## Human-Centric

MAVIS exists to assist.

Not replace.

Not control.

Not automate everything.

The user remains in control.

---

# Repository Layout

```

MAVIS/

config/
data/
docs/
logs/
memory/
plugins/

src/
└── mavis/

tests/

```

---

# Source Layout

```

src/
└── mavis/

__init__.py
__main__.py

main.py
bootstrap.py
app.py

core/
memory/
context/
planner/
executor/
plugins/
skills/
automation/
voice/
ui/
services/
models/

```

Each directory represents one major subsystem.

---

# Boot Process

Startup sequence

```

python -m mavis

↓

__main__.py

↓

main.py

↓

bootstrap.py

↓

Initialize runtime

↓

Load configuration

↓

Configure logging

↓

Create application

↓

Run lifecycle

↓

Application ready

```

---

# Bootstrap Responsibilities

Bootstrap is responsible for preparing MAVIS before it begins execution.

Responsibilities

- create runtime directories
- load configuration
- initialize logging
- prepare application
- perform startup validation
- return exit code

Bootstrap should never contain business logic.

---

# Application Layer

Main class

```

MavisApp

```

Responsibilities

- own application lifetime
- initialize subsystems
- start lifecycle
- coordinate shutdown

The application object is the highest-level runtime object.

---

# Core Package

The Core package provides foundational services used by every other subsystem.

```

core/

config.py
constants.py
events.py
lifecycle.py
logger.py
paths.py

```

Every module in the project may depend on Core.

Core should never depend on higher-level modules.

---

# Configuration

Configuration is loaded during bootstrap.

Current format

```

config/config.toml

```

Configuration philosophy

- Human readable
- Version controlled
- Easy to edit
- Minimal complexity

Future

Profiles

```

config/

default.toml

work.toml

gaming.toml

travel.toml

```

---

# Logging

Logging is initialized before any subsystem starts.

Every module receives its own logger.

Example

```

logger = get_logger(__name__)

```

Rules

Never use print()

Always use logging.

Logging levels

- DEBUG
- INFO
- WARNING
- ERROR
- CRITICAL

Future

- file logging
- rotating logs
- structured logs
- JSON logs

---

# Filesystem Layout

Project Root

```

config/

```

Application configuration.

```

logs/

```

Runtime logs.

```

memory/

```

Persistent memory storage.

```

data/

```

Application data.

```

plugins/

```

Installed plugins.

---

# Runtime Directories

Bootstrap automatically creates required directories.

Current

- config
- logs
- memory
- data
- plugins

This avoids manual setup.

---

# Constants

Application-wide constants belong inside

```

core/constants.py

```

Examples

- application name
- version
- default values
- default filenames

Avoid magic strings throughout the project.

---

# Paths

Filesystem paths belong inside

```

core/paths.py

```

No other module should manually construct project paths.

Benefits

- Centralized management
- Easier refactoring
- Cross-platform support

---

# Lifecycle

Lifecycle represents the running state of MAVIS.

Initial lifecycle

```

Created

↓

Initializing

↓

Running

↓

Stopping

↓

Stopped

```

Future states

- Restarting
- Updating
- Sleeping
- Suspended
- Error Recovery

Lifecycle transitions should always be logged.

---

# Error Handling

Principles

Recover whenever possible.

Fail gracefully.

Never silently ignore errors.

Every unexpected exception should be logged.

Critical startup failures should stop initialization.

---

# Dependency Philosophy

Keep dependencies minimal.

Prefer Python standard library whenever possible.

Only add third-party libraries when they provide significant value.

Reasons

- Faster installation
- Smaller footprint
- Easier maintenance
- Fewer security risks

Current external tools

- uv
- Ruff
- pre-commit

Everything else currently uses the Python standard library.

---

# Python and Rust

Primary language

Python

Reasons

- AI ecosystem
- Rapid development
- Excellent libraries

Rust will be used only where it provides measurable benefits.

Examples

- high-performance indexing
- vector search
- image processing
- encryption
- performance-critical algorithms

Python remains the orchestration language.

Rust acts as an accelerator.

---

# Separation of Responsibilities

Each subsystem should answer one question.

Core

"How does MAVIS run?"

Memory

"What does MAVIS remember?"

Context

"What is happening right now?"

Planner

"What should happen?"

Executor

"How do we perform it?"

Voice

"How do we communicate?"

UI

"How do we present information?"

Plugins

"How do we extend MAVIS?"

This separation should remain throughout the lifetime of the project.

# Memory Architecture

Memory is one of the most important subsystems in MAVIS.

Unlike a chatbot that only remembers the current conversation,
MAVIS maintains multiple memory layers with different responsibilities.

Each memory layer is independent.

A Memory Manager coordinates them.

---

# Memory Layers

```
                    Memory Manager
                           │
      ┌──────────┬──────────┬──────────┬──────────┐
      ▼          ▼          ▼          ▼          ▼
 Permanent   Long-Term   Episodic    Session   Working
```

Every layer answers a different question.

---

# Working Memory

Purpose

Contains information actively required to complete the current task.

Examples

- current objective
- current command
- active reasoning
- temporary variables
- planner state

Characteristics

- Very small
- Fast access
- Frequently updated
- Cleared after task completion

Think of this as MAVIS's "RAM."

---

# Session Memory

Purpose

Stores information relevant only to the current application session.

Examples

- conversation history
- current applications
- temporary reminders
- recent searches

Characteristics

- Exists until MAVIS exits
- Cleared on restart
- Larger than Working Memory

---

# Episodic Memory

Purpose

Stores important events.

Examples

- completed tasks
- conversations
- project milestones
- installation history
- major user actions

Examples

```
Created Sentinel project

Installed Niri

Finished MAVIS Phase 3

Changed preferred AI model
```

Future

Episodes may receive importance scores.

Higher importance means longer retention.

---

# Long-Term Memory

Purpose

Knowledge accumulated over time.

Examples

- user preferences
- learned workflows
- favorite applications
- preferred writing style
- recurring habits

Characteristics

Persistent.

Searchable.

May grow indefinitely.

Future

Semantic retrieval.

Vector indexing.

Knowledge summarization.

---

# Permanent Memory

Purpose

Critical identity information.

Examples

- application identity
- trusted plugins
- device registration
- permanent settings
- user-defined rules

Permanent memory should rarely change.

Changing permanent memory may require confirmation.

---

# Memory Manager

The Memory Manager provides a single interface to every memory layer.

Other subsystems should never access memory directly.

Instead:

```
Planner

↓

Memory Manager

↓

Appropriate Memory Layer
```

Responsibilities

- routing
- reading
- writing
- searching
- pruning
- summarizing
- synchronization

Future

Memory compression

Semantic search

Automatic cleanup

---

# Memory Rules

Working Memory

Never persistent.

Session Memory

Lives only during runtime.

Episodic Memory

Stores events.

Long-Term Memory

Stores knowledge.

Permanent Memory

Stores identity.

No layer should duplicate another.

---

# Context Engine

Purpose

The Context Engine determines what is happening right now.

Rather than simply answering a prompt, MAVIS should understand context.

Context is generated continuously.

---

# Context Sources

```
Conversation

Memory

Filesystem

Clipboard

Running Applications

Notifications

Calendar

Clock

Location (future)

Active Window

Network

Battery

Power State
```

Every source contributes to the current context.

---

# Unified Context

Instead of querying ten subsystems separately,
other modules request one Context object.

```
Planner

↓

Context Engine

↓

Unified Context
```

Benefits

- simpler APIs
- consistent reasoning
- centralized context building

---

# Context Lifecycle

```
Collect

↓

Validate

↓

Merge

↓

Prioritize

↓

Publish

↓

Consume
```

Every update produces a new context snapshot.

---

# Event System

The Event Bus allows subsystems to communicate without direct dependencies.

Example

```
Plugin Installed

↓

Publish Event

↓

Event Bus

↓

Interested Modules Receive Notification
```

This reduces coupling.

---

# Event Philosophy

Publishers never know who receives events.

Subscribers never know who produced them.

Only the Event Bus connects them.

---

# Example Events

System

```
ApplicationStarted

ApplicationStopping

ApplicationStopped
```

Memory

```
MemoryCreated

MemoryUpdated

MemoryDeleted
```

Plugins

```
PluginInstalled

PluginLoaded

PluginDisabled
```

Voice

```
ListeningStarted

ListeningStopped

SpeechRecognized
```

Automation

```
ReminderTriggered

AutomationCompleted
```

Future

Custom plugin events.

---

# Event Bus Responsibilities

- publish events
- subscribe handlers
- unsubscribe handlers
- dispatch asynchronously
- logging
- error isolation

---

# Planner

Purpose

Convert goals into executable plans.

Planner never performs work.

Planner only thinks.

Example

User says

```
Summarize this PDF
```

Planner creates

```
Open File

↓

Read PDF

↓

Extract Text

↓

Summarize

↓

Display Result
```

Executor performs it.

---

# Planner Responsibilities

- task decomposition
- dependency ordering
- validation
- scheduling
- retry policy
- cancellation

Planner should remain deterministic.

---

# Task Graph

Every plan becomes a task graph.

```
Goal

↓

Task A

↓

Task B

↓

Task C

↓

Finished
```

Future

Parallel execution.

Conditional branches.

Priority queues.

---

# Executor

Purpose

Execute planner output.

Executor performs actions.

Planner never executes.

---

# Executor Responsibilities

- run tasks
- monitor progress
- collect results
- rollback failures
- retry operations
- publish events

---

# Planner vs Executor

Planner

Answers

"What should happen?"

Executor

Answers

"How do we perform it?"

Keeping them separate greatly improves maintainability.

---

# Services

Services provide reusable functionality.

Examples

- filesystem
- networking
- AI providers
- OCR
- database
- notifications

Services should contain reusable code.

Business logic belongs elsewhere.

---

# Models

Models define shared data structures.

Examples

```
Context

MemoryEntry

Task

Plan

Plugin

Conversation

Event
```

Models should remain lightweight.

They should not contain business logic.

---

# Subsystem Communication

Preferred communication path

```
Subsystem

↓

Event Bus

↓

Interested Subsystems
```

Direct communication should only occur when ownership is clear.

---

# Dependency Direction

Dependencies should always point downward.

```
UI

↓

Planner

↓

Executor

↓

Services

↓

Core
```

Core should never depend on UI.

Memory should never depend on Voice.

Planner should not depend on Orb UI.

This prevents circular dependencies.

---

# Future Improvements

- Async event dispatcher
- Event priorities
- Event tracing
- Event replay
- Distributed events
- Remote plugin events
- Event debugging tools

# Plugin Architecture

One of MAVIS's primary design goals is extensibility.

Core functionality should remain small, stable, and maintainable.

Additional functionality should be implemented as plugins whenever practical.

---

# Plugin Philosophy

The core application should never become bloated.

If a feature can exist independently, it should become a plugin.

Examples

- Weather
- Spotify
- Calendar
- GitHub
- Home Assistant
- Docker
- Kubernetes
- Obsidian
- VS Code
- Discord

The core remains responsible only for providing the framework.

---

# Plugin Lifecycle

Every plugin follows the same lifecycle.

```
Discover

↓

Validate

↓

Load

↓

Initialize

↓

Register Events

↓

Running

↓

Unload

↓

Shutdown
```

---

# Plugin Structure

Example

```
plugins/

weather/

manifest.toml

plugin.py

assets/

README.md
```

Every plugin should be self-contained.

---

# Plugin Manifest

Every plugin must include metadata.

Example

```
Name

Version

Author

Description

Permissions

Entry Point

Dependencies
```

Future additions

- minimum MAVIS version
- supported operating systems
- plugin signature
- update channel

---

# Plugin Manager

Responsibilities

- discover plugins
- validate manifests
- load plugins
- unload plugins
- enable plugins
- disable plugins
- update plugins
- dependency resolution

The Plugin Manager is the only subsystem allowed to directly manage plugins.

---

# Plugin API

Plugins should interact with MAVIS through public APIs.

Plugins must never modify internal objects directly.

Examples

```
Memory API

Event API

Logging API

Notification API

Configuration API

Automation API
```

Stable APIs reduce breaking changes.

---

# Plugin Permissions

Every plugin declares permissions.

Examples

```
Filesystem

Internet

Clipboard

Notifications

Microphone

Camera

AI Access

Automation

Terminal
```

Sensitive permissions require explicit user approval.

---

# Plugin Sandboxing

Future versions should support isolated plugin execution.

Goals

- crash isolation
- security
- permission enforcement
- independent updates

---

# Skills

Skills are user-facing capabilities.

Unlike plugins, skills represent actions.

Examples

```
Open Browser

Create Reminder

Summarize Document

Read Clipboard

Take Screenshot

Shutdown Computer
```

Multiple plugins may provide skills.

---

# Skill Registry

Responsibilities

- discover skills
- register skills
- resolve conflicts
- expose searchable list

Future

Natural-language skill matching.

---

# Voice Architecture

Voice is a first-class subsystem.

Voice should remain optional.

MAVIS must work equally well without voice.

---

# Voice Pipeline

```
Wake Word

↓

Voice Activity Detection

↓

Speech-to-Text

↓

Intent Recognition

↓

Planner

↓

Executor

↓

Text-to-Speech
```

Each stage is independent.

---

# Voice Design Principles

- interruptible
- low latency
- offline capable
- modular
- provider agnostic

Future providers should be interchangeable.

---

# Orb UI

The Orb is MAVIS's visual identity.

It is not merely an icon.

It represents MAVIS's current state.

---

# Orb States

```
Idle

Listening

Thinking

Speaking

Working

Notification

Warning

Error

Sleeping
```

Animations should remain subtle.

The Orb should communicate state without becoming distracting.

---

# Workspace Mode

The workspace expands beyond the floating orb.

Purpose

- conversations
- planning
- debugging
- document viewing
- plugin management

The Orb becomes the entry point.

The workspace becomes the productivity interface.

---

# Companion Display Mode

Future versions may dedicate an entire secondary monitor to MAVIS.

Possible widgets

- Orb
- Calendar
- Tasks
- Weather
- System Status
- Notifications
- Running Automations
- Daily Agenda

This mode turns MAVIS into a persistent desktop companion.

---

# User Interface Principles

The interface should be

- minimal
- calm
- responsive
- unobtrusive
- informative

Visual noise should be avoided.

The UI should support long work sessions without fatigue.

---

# AI Provider Architecture

The AI subsystem should not depend on one model.

Instead, MAVIS communicates through a provider abstraction.

```
Planner

↓

AI Interface

↓

Provider
```

Possible providers

- Local LLM
- Ollama
- llama.cpp
- OpenAI
- Anthropic
- Google
- Future providers

Changing providers should require configuration only.

---

# Prompt System

Prompt construction belongs in one dedicated subsystem.

Responsibilities

- inject context
- inject memory
- inject user preferences
- maintain prompt templates

Prompt generation should never be scattered throughout the codebase.

---

# Native Operating System Integration

MAVIS should integrate naturally with the host operating system.

Linux

- Notifications
- Clipboard
- Media Controls
- Power Management
- Wayland/X11
- File Manager
- Default Applications

Windows

- Notifications
- Explorer
- Clipboard
- Power APIs

macOS

- Notifications
- Finder
- Spotlight
- Clipboard

Platform-specific code should remain isolated.

---

# Automation Engine

Automation enables proactive assistance.

Examples

```
Battery Reminder

Hydration Reminder

Stretch Reminder

Sleep Reminder

Calendar Alert

Daily Summary
```

Automation must remain predictable.

Unexpected autonomous behavior should be avoided.

---

# Security Architecture

Security is mandatory.

Principles

- least privilege
- explicit permission
- auditability
- transparency

MAVIS should never silently perform privileged operations.

---

# Permission Levels

Level 1

Read-only

Examples

- weather
- clock
- clipboard

Level 2

Confirmation Required

Examples

- delete files
- install packages
- run scripts

Level 3

Restricted

Examples

- firmware
- disk formatting
- destructive actions

---

# Rust Integration Strategy

Python remains the orchestration language.

Rust accelerates performance-critical components.

Candidate modules

- semantic indexing
- vector search
- cryptography
- image processing
- OCR acceleration
- compression
- memory search

Rust modules should expose clean Python APIs.

Business logic remains in Python.

---

# Packaging Strategy

Development

```
Source Repository

↓

uv

↓

Editable Installation
```

Release

```
Source

↓

Build

↓

Package

↓

Installer

↓

User
```

Future distribution

- Linux packages
- Windows installer
- macOS installer

---

# Testing Architecture

Every subsystem should support independent testing.

Testing layers

```
Unit

↓

Integration

↓

System

↓

Regression

↓

Performance
```

Future

Continuous Integration.

---

# Documentation Philosophy

Documentation is treated as part of the codebase.

Rules

Every subsystem requires documentation.

Every architectural change updates ARCHITECTURE.md.

Every coding rule updates ENGINEERING.md.

Every feature updates ROADMAP.md.

Every release updates CHANGELOG.md.

Documentation should never significantly lag behind implementation.

---

# Architectural Rules

1. Prefer composition over inheritance.

2. Keep modules focused on one responsibility.

3. Avoid circular dependencies.

4. Prefer interfaces over concrete implementations.

5. Keep external dependencies minimal.

6. Separate planning from execution.

7. Keep AI providers interchangeable.

8. Use the standard library whenever practical.

9. Prefer explicit behavior over hidden magic.

10. Maintain backward compatibility whenever possible.

---

# Long-Term Vision

MAVIS should evolve into a reliable desktop operating companion.

The architecture must allow new capabilities to be added without requiring large-scale rewrites.

The goal is not to build the largest AI assistant.

The goal is to build one that remains understandable, maintainable, extensible, and dependable over many years of development.

---

End of Architecture Document.