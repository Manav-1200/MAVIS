"""
Application logging.

Purpose
-------
Provide a centralized logger for the entire MAVIS application.

Design
------
All MAVIS modules should obtain loggers through this module
instead of configuring logging themselves. This ensures
consistent formatting and behavior across the project.
"""

from __future__ import annotations

import logging


def setup_logging() -> None:
    """
    Configure the application's logging system.

    This function should only be called once during startup.
    """

    logging.basicConfig(
        level=logging.INFO,
        format="%(asctime)s | %(levelname)-8s | %(name)s | %(message)s",
        datefmt="%H:%M:%S",
    )


def get_logger(name: str) -> logging.Logger:
    """
    Return a configured logger.

    Parameters
    ----------
    name
        Name of the logger.

    Returns
    -------
    logging.Logger
        Configured logger instance.
    """

    return logging.getLogger(name)
