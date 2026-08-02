"""
Application-wide constants.

This module contains immutable values shared across the project.

Keeping constants in one place avoids duplicated string literals and
makes future changes significantly easier.
"""

from __future__ import annotations

# ---------------------------------------------------------------------
# Application Information
# ---------------------------------------------------------------------

APP_NAME = "MAVIS"
APP_FULL_NAME = "Modular Autonomous Virtual Intelligence System"
APP_VERSION = "0.1.0"

# ---------------------------------------------------------------------
# Directory Names
# ---------------------------------------------------------------------

CONFIG_DIR_NAME = "config"
DATA_DIR_NAME = "data"
LOG_DIR_NAME = "logs"
MEMORY_DIR_NAME = "memory"
PLUGIN_DIR_NAME = "plugins"

# ---------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------

DEFAULT_CONFIG_FILE = "config.toml"

# ---------------------------------------------------------------------
# Logging
# ---------------------------------------------------------------------

DEFAULT_LOG_LEVEL = "INFO"
DEFAULT_LOG_FILE = "mavis.log"
