"""
Main application entry point.

Purpose
-------
Provides the application's public startup function.

Design
------
Every startup request eventually arrives here.
This module delegates initialization to the bootstrap process.
"""

from __future__ import annotations

from mavis.bootstrap import bootstrap


def main() -> int:
    """
    Start the MAVIS application.

    Returns
    -------
    int
        Process exit status.
    """

    return bootstrap()
