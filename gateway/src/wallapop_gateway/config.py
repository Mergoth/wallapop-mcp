"""Gateway settings, read once from the environment."""

import ipaddress
import os
import shlex
from collections.abc import Mapping
from dataclasses import dataclass
from pathlib import Path
from urllib.parse import urlparse


class ConfigError(Exception):
    """The gateway refuses to start with this configuration."""


@dataclass(frozen=True)
class Settings:
    host: str
    port: int
    resource_url: str
    issuer_url: str
    google_client_id: str
    google_client_secret: str
    owner_email: str
    state_dir: Path
    static_token: str | None
    upstream_command: str
    upstream_args: tuple[str, ...]
    upstream_env: Mapping[str, str]


def _is_loopback(host: str) -> bool:
    if host == "localhost":
        return True
    try:
        return ipaddress.ip_address(host).is_loopback
    except ValueError:
        return False


def _origin(url: str) -> str:
    parsed = urlparse(url)
    return f"{parsed.scheme}://{parsed.netloc}"


def _required(env: Mapping[str, str], name: str) -> str:
    value = env.get(name, "").strip()
    if not value:
        raise ConfigError(f"{name} is not set")
    return value


def load_settings(env: Mapping[str, str] | None = None) -> Settings:
    """``MCP_RESOURCE_URL`` is the MCP URL exactly as clients enter it, ``/mcp`` included.

    It defaults to the bind address only on loopback; behind a proxy a loopback default would
    tell remote clients the resource lives on 127.0.0.1, so it is required there.
    """
    env = os.environ if env is None else env
    host = env.get("MCP_HOST") or "127.0.0.1"
    raw_port = env.get("MCP_PORT") or "8000"
    try:
        port = int(raw_port)
    except ValueError:
        raise ConfigError(f"MCP_PORT={raw_port!r} is not an integer") from None

    resource_url = env.get("MCP_RESOURCE_URL")
    if not resource_url:
        if not _is_loopback(host):
            raise ConfigError(f"MCP_RESOURCE_URL is required when MCP_HOST={host!r}")
        resource_url = f"http://{host}:{port}/mcp"
    issuer_url = env.get("MCP_ISSUER_URL") or _origin(resource_url)

    owner_email = _required(env, "MCP_OWNER_EMAIL")
    if "@" not in owner_email:
        raise ConfigError(f"MCP_OWNER_EMAIL={owner_email!r} is not an email address")

    return Settings(
        host=host,
        port=port,
        resource_url=resource_url,
        issuer_url=issuer_url,
        google_client_id=_required(env, "GOOGLE_CLIENT_ID"),
        google_client_secret=_required(env, "GOOGLE_CLIENT_SECRET"),
        owner_email=owner_email,
        state_dir=Path(_required(env, "MCP_STATE_DIR")),
        static_token=env.get("MCP_AUTH_TOKEN") or None,
        upstream_command=env.get("WALLAPOP_MCP_COMMAND") or "wallapop-mcp",
        upstream_args=tuple(shlex.split(env.get("WALLAPOP_MCP_ARGS", ""))),
        # The SDK spawns the child with a minimal environment; pass its own settings through.
        upstream_env={k: v for k, v in env.items() if k.startswith("WALLAPOP_")},
    )
