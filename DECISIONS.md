# Architectural Decision Record

---

## ADR-001

Title

Single Companion Identity

Status

Accepted

Date

2026-08-03

Decision

MAVIS will present one persistent identity regardless of how many AI
providers or internal components are used.

Reason

Users build trust with one companion.

Multiple identities create confusion.

Alternatives Considered

Multi-agent systems

CrewAI

AutoGen

Role-based assistants

Consequences

Positive

Simple UX.

Stable personality.

Consistent memory.

Negative

Internal specialization becomes less visible.

Canonical References

203
160
124

---

## ADR-002

Title

Planner and Executor Separation

