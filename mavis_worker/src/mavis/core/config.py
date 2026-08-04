"""
MAVIS Configuration Manager

Loads, validates, and provides access to the MAVIS configuration.
"""

from __future__ import annotations

import tomllib
from pathlib import Path
from typing import Any

from mavis.core.constants import DEFAULT_CONFIG_FILE
from mavis.core.paths import CONFIG_DIR


class ConfigurationError(Exception):
    """Raised when the configuration is invalid."""


class AppConfig:
    """Loads and manages the MAVIS configuration."""

    def __init__(self, config_path: Path | None = None) -> None:
        self._config_path = config_path or CONFIG_DIR / DEFAULT_CONFIG_FILE
        self._data: dict[str, Any] = {}

    @property
    def path(self) -> Path:
        return self._config_path

    @property
    def data(self) -> dict[str, Any]:
        return self._data

    def load(self) -> None:
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
        self.load()

    def get(self, key: str, default: Any = None) -> Any:
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
        return {
            "application": {"debug": False},
            "logging": {"level": "INFO"},
        }

    def _validate(self) -> None:
        valid_levels = {"DEBUG", "INFO", "WARNING", "ERROR", "CRITICAL"}
        level = self.get("logging.level")
        if level not in valid_levels:
            raise ConfigurationError(f"Invalid logging level: {level}")


def load_config() -> AppConfig:
    """Load and return the MAVIS configuration."""
    cfg = AppConfig()
    cfg.load()
    return cfg


# Global instance for convenience
config = AppConfig()
config.load()
