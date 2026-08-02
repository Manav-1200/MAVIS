"""
Application bootstrap.

Purpose
-------
Prepare everything required before MAVIS starts running.

Design
------
Bootstrap creates required runtime directories, loads configuration,
initializes logging, and starts the application.
"""

from __future__ import annotations

from mavis.app import MavisApp
from mavis.core.config import load_config
from mavis.core.logger import get_logger, setup_logging
from mavis.core.paths import (
    CONFIG_DIRECTORY,
    DATA_DIRECTORY,
    LOG_DIRECTORY,
    MEMORY_DIRECTORY,
    PLUGIN_DIRECTORY,
)


def bootstrap() -> int:
    """
    Initialize and start MAVIS.

    Returns
    -------
    int
        Process exit status.
    """

    # Ensure runtime directories exist.
    for directory in (
        CONFIG_DIRECTORY,
        DATA_DIRECTORY,
        LOG_DIRECTORY,
        MEMORY_DIRECTORY,
        PLUGIN_DIRECTORY,
    ):
        directory.mkdir(parents=True, exist_ok=True)

    config = load_config()

    setup_logging()

    logger = get_logger(__name__)

    logger.info("Initializing %s...", config.app_name)

    app = MavisApp(config)

    return app.run()
