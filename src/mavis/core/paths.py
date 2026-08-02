"""
Application paths.

Purpose
-------
Provide a centralized definition of filesystem paths used by MAVIS.

Design
------
All filesystem locations are defined relative to the project root.
Future versions may support platform-specific data directories,
but the rest of the application should always obtain paths through
this module instead of constructing them manually.
"""

from __future__ import annotations

from pathlib import Path

# Root of the MAVIS project.
PROJECT_ROOT = Path(__file__).resolve().parents[3]

# Source directory.
SRC_DIRECTORY = PROJECT_ROOT / "src"

# Runtime directories.
DATA_DIRECTORY = PROJECT_ROOT / "data"
LOG_DIRECTORY = PROJECT_ROOT / "logs"
MEMORY_DIRECTORY = PROJECT_ROOT / "memory"
PLUGIN_DIRECTORY = PROJECT_ROOT / "plugins"
CONFIG_DIRECTORY = PROJECT_ROOT / "config"

# Primary configuration file.
CONFIG_FILE = CONFIG_DIRECTORY / "config.toml"
