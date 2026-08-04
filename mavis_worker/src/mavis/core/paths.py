"""
MAVIS Runtime Paths

Defines all runtime filesystem paths used by MAVIS.

This module:
- Computes paths only.
- Never creates directories.
- Has no side effects.
"""

from __future__ import annotations

import os
from pathlib import Path
from typing import Final

from mavis.core.constants import (
    CACHE_DIR_NAME,
    CONFIG_DIR_NAME,
    DATA_DIR_NAME,
    ENV_CONFIG_DIR,
    ENV_DATA_DIR,
    LOG_DIR_NAME,
    MEMORY_DIR_NAME,
    PLUGIN_DIR_NAME,
    TEMP_DIR_NAME,
)

# ============================================================================
# Base Paths
# ============================================================================

# User's home directory.
HOME_DIR: Final[Path] = Path.home()

# Root configuration directory.
# Can be overridden using MAVIS_CONFIG_DIR.
CONFIG_ROOT: Final[Path] = Path(
    os.getenv(ENV_CONFIG_DIR, HOME_DIR / ".config" / "mavis")
).expanduser()

# Root data directory.
# Can be overridden using MAVIS_DATA_DIR.
DATA_ROOT: Final[Path] = Path(
    os.getenv(ENV_DATA_DIR, HOME_DIR / ".local" / "share" / "mavis")
).expanduser()

# ============================================================================
# Runtime Directories
# ============================================================================

# Configuration files.
CONFIG_DIR: Final[Path] = CONFIG_ROOT / CONFIG_DIR_NAME

# Application data.
DATA_DIR: Final[Path] = DATA_ROOT / DATA_DIR_NAME

# Temporary cache.
CACHE_DIR: Final[Path] = DATA_ROOT / CACHE_DIR_NAME

# Log files.
LOG_DIR: Final[Path] = DATA_ROOT / LOG_DIR_NAME

# Memory storage.
MEMORY_DIR: Final[Path] = DATA_ROOT / MEMORY_DIR_NAME

# Installed plugins.
PLUGIN_DIR: Final[Path] = DATA_ROOT / PLUGIN_DIR_NAME

# Temporary runtime files.
TEMP_DIR: Final[Path] = DATA_ROOT / TEMP_DIR_NAME
