"""
src/mavis/core/config.py

MAVIS Configuration Manager

Loads, validates, and provides access to the MAVIS configuration.

Responsibilities:
- Load configuration from TOML.
- Apply default values.
- Validate configuration values.
- Provide read-only access.
"""

from __future__ import annotations

import tomllib
from pathlib import Path
from typing import Any

from mavis.core.constants import DEFAULT_CONFIG_FILE
from mavis.core.paths import CONFIG_DIR

# ============================================================================
# Exceptions
# ============================================================================


class ConfigurationError(Exception):
    """Raised when the configuration is invalid."""


# ============================================================================
# Configuration Manager
# ============================================================================


class Config:
    """Loads and manages the MAVIS configuration."""

    def __init__(self, config_path: Path | None = None) -> None:
        """
        Initialize the configuration manager.

        Args:
            config_path:
                Optional path to the configuration file.
        """
        self._config_path = config_path or CONFIG_DIR / DEFAULT_CONFIG_FILE
        self._data: dict[str, Any] = {}

    @property
    def path(self) -> Path:
        """Return the configuration file path."""
        return self._config_path

    @property
    def data(self) -> dict[str, Any]:
        """Return the loaded configuration."""
        return self._data

    def load(self) -> None:
        """
        Load the configuration from disk.

        If the configuration file does not exist,
        the default configuration is used.

        Raises:
            ConfigurationError:
                If the configuration cannot be loaded
                or fails validation.
        """
        if not self._config_path.exists():
            self._data = self._default_config()
            return

        try:
            with self._config_path.open("rb") as file:
                self._data = tomllib.load(file)
        except Exception as exc:
            raise ConfigurationError(f"Failed to load configuration: {exc}") from exc

        self._validate()

    def reload(self) -> None:
        """Reload the configuration."""
        self.load()

    def get(self, key: str, default: Any = None) -> Any:
        """
        Return a configuration value.

        Supports dotted keys.

        Example:
            config.get("logging.level")
        """
        value: Any = self._data

        for part in key.split("."):
            if not isinstance(value, dict):
                return default

            value = value.get(part)

            if value is None:
                return default

        return value

    @staticmethod
    def _default_config() -> dict[str, Any]:
        """Return the default configuration."""

        return {
            "application": {
                "debug": False,
            },
            "logging": {
                "level": "INFO",
            },
        }

    def _validate(self) -> None:
        """
        Validate the loaded configuration.

        Raises:
            ConfigurationError:
                If a configuration value is invalid.
        """
        valid_levels = {
            "DEBUG",
            "INFO",
            "WARNING",
            "ERROR",
            "CRITICAL",
        }

        level = self.get("logging.level")

        if level not in valid_levels:
            raise ConfigurationError(f"Invalid logging level: {level}")


# ============================================================================
# Global Configuration Instance
# ============================================================================

config = Config()
config.load()
