"""
Application event bus.

Purpose
-------
Provide a simple publish/subscribe system that allows MAVIS
subsystems to communicate without depending directly on one another.

Design
------
Components publish events to the event bus instead of calling
each other directly. This keeps the architecture modular and
reduces coupling between subsystems.
"""

from __future__ import annotations

from collections.abc import Callable
from typing import Any

EventHandler = Callable[..., None]


class EventBus:
    """
    Simple in-process event bus.
    """

    def __init__(self) -> None:
        """Create an empty event bus."""

        self._listeners: dict[str, list[EventHandler]] = {}

    def subscribe(self, event: str, handler: EventHandler) -> None:
        """
        Register a handler for an event.
        """

        self._listeners.setdefault(event, []).append(handler)

    def publish(self, event: str, **data: Any) -> None:
        """
        Publish an event to all registered listeners.
        """

        for handler in self._listeners.get(event, []):
            handler(**data)
