"""
Application lifecycle management.

Purpose
-------
Manage the startup and shutdown sequence of the MAVIS application.

Design
------
The lifecycle manager is responsible for controlling the
application's execution state. As MAVIS grows, startup and
shutdown logic for subsystems will be implemented here.
"""

from __future__ import annotations

from mavis.core.logger import get_logger


class LifecycleManager:
    """
    Controls the application's lifecycle.
    """

    def __init__(self) -> None:
        """
        Create a new lifecycle manager.
        """

        self.logger = get_logger(__name__)

    def start(self) -> None:
        """
        Start the application lifecycle.
        """

        self.logger.info("Starting application lifecycle.")

    def stop(self) -> None:
        """
        Stop the application lifecycle.
        """

        self.logger.info("Stopping application lifecycle.")
