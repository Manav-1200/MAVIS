"""
MAVIS Core Constants

This module defines immutable constants used throughout the MAVIS runtime.

Rules:
- No runtime logic.
- No filesystem access.
- No environment variable access.
- No mutable globals.
"""

from typing import Final

# ============================================================================
# Application
# ============================================================================

APP_NAME: Final[str] = "MAVIS"
APP_VERSION: Final[str] = "0.1.0"
APP_AUTHOR: Final[str] = "Manav Neupane"
APP_DESCRIPTION: Final[str] = "Modular Autonomous Virtual Intelligence System"

# ============================================================================
# Configuration
# ============================================================================

DEFAULT_CONFIG_FILE: Final[str] = "config.toml"

ENV_CONFIG_DIR: Final[str] = "MAVIS_CONFIG_DIR"
ENV_DATA_DIR: Final[str] = "MAVIS_DATA_DIR"

# ============================================================================
# Directory Names
# ============================================================================

CONFIG_DIR_NAME: Final[str] = "config"
DATA_DIR_NAME: Final[str] = "data"
CACHE_DIR_NAME: Final[str] = "cache"
LOG_DIR_NAME: Final[str] = "logs"
MEMORY_DIR_NAME: Final[str] = "memory"
PLUGIN_DIR_NAME: Final[str] = "plugins"
TEMP_DIR_NAME: Final[str] = "temp"

# ============================================================================
# Logging
# ============================================================================

LOGGER_NAME: Final[str] = "mavis"

LOG_FILE_NAME: Final[str] = "mavis.log"

DEFAULT_LOG_LEVEL: Final[str] = "INFO"

# ============================================================================
# Runtime
# ============================================================================

DEFAULT_ENCODING: Final[str] = "utf-8"

SHUTDOWN_TIMEOUT_SECONDS: Final[int] = 10
EVENT_TIMEOUT_SECONDS: Final[int] = 30

# ============================================================================
# Exit Codes
# ============================================================================

EXIT_SUCCESS: Final[int] = 0
EXIT_FAILURE: Final[int] = 1
EXIT_CONFIGURATION_ERROR: Final[int] = 2

# ============================================================================
# Service Names
# ============================================================================

CONFIG_SERVICE: Final[str] = "config"
EVENT_BUS_SERVICE: Final[str] = "event_bus"
LOGGER_SERVICE: Final[str] = "logger"
LIFECYCLE_SERVICE: Final[str] = "lifecycle"
FILESYSTEM_SERVICE: Final[str] = "filesystem"
