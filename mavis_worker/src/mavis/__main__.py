"""
Package execution entry point.

Purpose
-------
Allows MAVIS to be started with:

    python -m mavis

Design
------
This module contains no application logic.
Its only responsibility is to delegate execution to
the application's public entry point.
"""

from __future__ import annotations

from mavis.main import main

if __name__ == "__main__":
    raise SystemExit(main())
