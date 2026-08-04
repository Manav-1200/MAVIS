"""
Application bootstrap.

Prepares everything required before MAVIS starts running.
"""

from __future__ import annotations

from mavis.app import MavisApp
from mavis.core.config import load_config
from mavis.core.logger import get_logger, setup_logging
from mavis.core.paths import CONFIG_DIR, DATA_DIR, LOG_DIR, MEMORY_DIR, PLUGIN_DIR


def bootstrap() -> int:
    """Initialize and start MAVIS."""

    for directory in (CONFIG_DIR, DATA_DIR, LOG_DIR, MEMORY_DIR, PLUGIN_DIR):
        directory.mkdir(parents=True, exist_ok=True)

    config = load_config()
    setup_logging()
    logger = get_logger(__name__)
    logger.info("Initializing %s...", config.path)

    app = MavisApp(config)
    return app.run()
