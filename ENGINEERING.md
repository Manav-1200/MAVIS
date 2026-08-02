# MAVIS Engineering Handbook

> Modular Autonomous Virtual Intelligence System (MAVIS)
>
> This document defines the engineering standards for the MAVIS project.
>
> Every contributor should follow these guidelines to keep the codebase
> consistent, maintainable, and production-ready.

---

# Philosophy

MAVIS is a long-term engineering project.

The goal is not to write code quickly.

The goal is to build software that remains understandable,
maintainable, and extensible years from now.

Engineering principles take priority over convenience.

---

# Core Principles

Every contribution should satisfy the following principles.

## Correctness

Correct code is always preferred over clever code.

---

## Readability

Code is read more often than it is written.

Optimize for readability.

---

## Simplicity

Prefer the simplest solution that satisfies the requirements.

Avoid unnecessary abstraction.

---

## Maintainability

Assume another engineer will read the code years later.

Write code for them.

---

## Consistency

Consistency is more valuable than personal preference.

Follow existing project conventions.

---

# Documentation First

Documentation is not optional.

Major architectural decisions must be documented before or alongside
implementation.

Required updates

Feature

↓

ROADMAP.md

Architecture change

↓

ARCHITECTURE.md

Coding standard

↓

ENGINEERING.md

Project overview

↓

README.md

Release

↓

CHANGELOG.md

AI guidance

↓

AI_CONTEXT.md

---

# Repository Structure

```

MAVIS/

config/
data/
logs/
memory/
plugins/

src/
tests/
docs/

```

Source code belongs only inside

```
src/
```

No executable code belongs in the repository root.

---

# Python Version

Current target

```
Python 3.14+
```

Use new language features when they improve readability.

Avoid compatibility hacks unless officially supported.

---

# Dependency Policy

External dependencies must be justified.

Ask:

Does the Python standard library already solve this?

If yes

Do not add a dependency.

Reasons

- fewer security risks
- faster installation
- easier maintenance
- smaller project

---

# Current Toolchain

Package management

```
uv
```

Formatter

```
ruff
```

Linting

```
ruff
```

Git hooks

```
pre-commit
```

Version control

```
git
```

---

# Code Style

Use

- type hints
- descriptive names
- short functions
- meaningful modules

Avoid

- magic numbers
- deeply nested code
- unnecessary globals
- duplicated logic

---

# Naming Conventions

Packages

```
lowercase
```

Modules

```
snake_case.py
```

Functions

```
snake_case()
```

Variables

```
snake_case
```

Constants

```
UPPER_CASE
```

Classes

```
PascalCase
```

Private members

```
_leading_underscore
```

---

# Comments

Good comments explain

WHY

not

WHAT

Bad

```
x += 1
# Increase x
```

Good

```
# Retry counter prevents infinite reconnect loops.
```

---

# Docstrings

Every public module

Every public class

Every public function

should have a docstring.

Use Google or NumPy style consistently.

Current project standard

NumPy style.

---

# Logging

Never use

```
print()
```

Use

```
logger.info()

logger.warning()

logger.error()

logger.exception()
```

Every module owns its own logger.

```
logger = get_logger(__name__)
```

---

# Error Handling

Never silently ignore exceptions.

Bad

```
except:
    pass
```

Good

```
except Exception as exc:
    logger.exception(...)
```

Recover when possible.

Fail gracefully.

---

# Project Layers

Higher layers may depend on lower layers.

Lower layers must never depend on higher layers.

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

---

# Circular Dependencies

Forbidden.

If two modules require each other,

the architecture is wrong.

Refactor.

---

# Single Responsibility Principle

Each module should answer one question.

Bad

```
config.py

Loads config

Creates UI

Starts network

Runs AI
```

Good

```
config.py

Loads configuration.
```

---

# Function Size

Prefer

20–40 lines.

Large functions usually indicate multiple responsibilities.

Split them.

---

# Class Design

Classes represent

objects

or

services.

Do not create classes merely to group functions.

---

# Composition

Prefer

Composition

over

Inheritance.

Inheritance only when there is a genuine "is-a" relationship.

---

# Global State

Avoid global mutable state.

Prefer dependency injection.

---

# Configuration

Never hardcode user-configurable values.

Use

```
config.toml
```

---

# Paths

Never manually build project paths.

Always use

```
core.paths
```

---

# Constants

Never duplicate string literals.

Use

```
core.constants
```

---

# Event System

Subsystems should communicate through events whenever practical.

Avoid direct coupling.

---

# Memory Access

Memory layers must never be accessed directly.

Always use

```
MemoryManager
```

---

# Plugin Development

Plugins should

- use public APIs
- declare permissions
- remain self-contained

Plugins must never modify internal MAVIS objects directly.

---

# Security

Never expose

- API keys
- passwords
- tokens

Never commit secrets.

Use

```
.env
```

for development.

---

# Git Workflow

Feature

↓

Branch

↓

Commit

↓

Review

↓

Merge

Commit messages

```
feat:

fix:

docs:

refactor:

test:

perf:

chore:
```

---

# Commits

Prefer small commits.

Each commit should represent one logical change.

Bad

```
Implemented everything
```

Good

```
feat(memory): add working memory manager
```

---

# Formatting

Formatting is automatic.

Run

```
ruff format
```

before committing.

---

# Linting

Run

```
ruff check
```

Fix warnings immediately.

---

# Pre-commit

Every commit should pass

```
pre-commit
```

before entering Git history.

---

# Testing Philosophy

Every important module should eventually have tests.

Testing pyramid

Unit

↓

Integration

↓

System

↓

Performance

---

# Performance

Do not optimize prematurely.

Measure first.

Optimize later.

---

# Rust

Rust is reserved for

performance-critical

components.

Business logic remains in Python.

---

# Reviews

Every review should ask

Is this

Correct?

Readable?

Maintainable?

Documented?

Testable?

Consistent?

---

# AI Contributions

AI-generated code is treated exactly like human-written code.

Every contribution must satisfy

- project architecture
- coding standards
- documentation standards

No exceptions.

---

# Future Contributors

Future developers should be able to understand the project without
reading old conversations.

The repository itself should explain the project.

Documentation is part of the product.

---

# Engineering Rule

If a feature is difficult to explain,

the design should be reconsidered.

Simple architectures survive.

Complex architectures eventually collapse.

Always build MAVIS so that future development becomes easier,
not harder.

---

End of Engineering Handbook.