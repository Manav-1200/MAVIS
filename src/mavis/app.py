"""
MAVIS application.

Purpose
-------
Defines the main application object responsible for coordinating
the major subsystems of MAVIS.

Design
------
The application acts as the central coordinator for MAVIS. It owns
shared services and, as the project grows, will manage components
such as memory, plugins, automation, the UI, and AI services.
"""

from __future__ import annotations

from mavis.core.config import AppConfig
from mavis.core.events import EventBus
from mavis.core.lifecycle import LifecycleManager
from mavis.core.logger import get_logger


class MavisApp:
    """
    Represents a running MAVIS application.
    """

    def __init__(self, config: AppConfig) -> None:
        """
        Create a new MAVIS application instance.

        Parameters
        ----------
        config
            The application's configuration.
        """

        self.config = config
        self.events = EventBus()
        self.lifecycle = LifecycleManager()
        self.logger = get_logger(__name__)

    def run(self) -> int:
        """
        Start the MAVIS application.

        Returns
        -------
        int
            Process exit status.
        """

        self.lifecycle.start()

        # Notify interested components that the application has started.
        self.events.publish(
            "app.started",
            version=self.config.version,
        )

        self.logger.info(
            "%s %s is now running.",
            self.config.app_name,
            self.config.version,
        )

        return 0
