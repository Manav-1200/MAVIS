"""
Application configuration.

Purpose
-------
Load and provide access to MAVIS runtime configuration.

Design
------
Configuration is loaded from ``config/config.toml``.

The configuration is parsed only once during application startup.
Subsequent calls return the cached configuration object.

If a value is missing, sensible defaults are used. This allows new
configuration options to be added in future versions without breaking
older configuration files.
"""

from __future__ import annotations

import tomllib
from dataclasses import dataclass

from mavis.core.constants import APP_NAME, APP_VERSION
from mavis.core.paths import CONFIG_FILE


@dataclass(slots=True)
class AppConfig:
    """
    Runtime configuration for MAVIS.
    """

    app_name: str
    version: str
    debug: bool
    log_level: str
    theme: str
    voice_enabled: bool
    ai_provider: str


# Cached configuration instance.
#
# The configuration is loaded only once during application startup.
# All subsequent calls to get_config() return this cached object.
_CONFIG: AppConfig | None = None


def load_config() -> AppConfig:
    """
    Load configuration from ``config/config.toml``.

    Returns
    -------
    AppConfig
        Loaded application configuration.
    """

    with CONFIG_FILE.open("rb") as file:
        config = tomllib.load(file)

    return AppConfig(
        app_name=config.get("application", {}).get("name", APP_NAME),
        version=config.get("application", {}).get("version", APP_VERSION),
        debug=config.get("application", {}).get("debug", False),
        log_level=config.get("logging", {}).get("level", "INFO"),
        theme=config.get("ui", {}).get("theme", "dark"),
        voice_enabled=config.get("voice", {}).get("enabled", True),
        ai_provider=config.get("ai", {}).get("provider", "local"),
    )


def get_config() -> AppConfig:
    """
    Return the application's runtime configuration.

    The configuration is loaded from disk only on the first call.
    Later calls return the cached configuration instance.

    Returns
    -------
    AppConfig
        The current application configuration.
    """

    global _CONFIG

    if _CONFIG is None:
        _CONFIG = load_config()

    return _CONFIG
