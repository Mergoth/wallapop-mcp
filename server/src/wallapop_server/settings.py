"""Settings for wallapop-server, read from environment."""

import os
import shlex
from collections.abc import Mapping
from dataclasses import dataclass
from pathlib import Path


class ConfigError(Exception):
    """Configuration error."""


@dataclass(frozen=True)
class Settings:
    upstream_token: str
    gateway_registration_token: str | None
    gateway_internal_url: str
    host: str
    port: int
    advertise_url: str
    command: str
    args: tuple[str, ...]
    child_env: Mapping[str, str]
    upload_dir: Path | None
    upload_secret: bytes | None
    no_register: bool

    @classmethod
    def from_env(cls, env: Mapping[str, str] | None = None) -> "Settings":
        if env is None:
            env = os.environ

        upstream_token = env.get("WALLAPOP_UPSTREAM_TOKEN", "").strip()
        if not upstream_token:
            raise ConfigError("WALLAPOP_UPSTREAM_TOKEN is required")
        if len(upstream_token) < 32:
            raise ConfigError(
                f"WALLAPOP_UPSTREAM_TOKEN must be at least 32 characters (got {len(upstream_token)})"
            )

        no_register = env.get("WALLAPOP_NO_REGISTER") == "1"
        gateway_registration_token: str | None = None
        if not no_register:
            raw_reg_token = env.get("GATEWAY_REGISTRATION_TOKEN", "").strip()
            if not raw_reg_token:
                raise ConfigError("GATEWAY_REGISTRATION_TOKEN is required")
            gateway_registration_token = raw_reg_token
        else:
            raw_reg_token = env.get("GATEWAY_REGISTRATION_TOKEN", "").strip()
            gateway_registration_token = raw_reg_token if raw_reg_token else None

        gateway_internal_url = env.get("GATEWAY_INTERNAL_URL") or "http://mcp-gateway:8081"
        host = env.get("WALLAPOP_HOST") or "127.0.0.1"

        raw_port = env.get("WALLAPOP_PORT") or "8000"
        try:
            port = int(raw_port)
        except ValueError:
            raise ConfigError(f"WALLAPOP_PORT={raw_port!r} is not an integer") from None

        advertise_url = env.get("WALLAPOP_ADVERTISE_URL") or "http://wallapop-mcp:8000/mcp"
        command = env.get("WALLAPOP_MCP_COMMAND") or "wallapop-mcp"
        raw_args = env.get("WALLAPOP_MCP_ARGS", "")
        args = tuple(shlex.split(raw_args))

        raw_upload_dir = env.get("UPLOAD_DIR")
        upload_dir: Path | None = None
        upload_secret: bytes | None = None
        if raw_upload_dir:
            upload_dir = Path(raw_upload_dir)
            raw_upload_secret = env.get("UPLOAD_SECRET")
            if not raw_upload_secret:
                raise ConfigError("UPLOAD_SECRET is required when UPLOAD_DIR is set")
            upload_secret = raw_upload_secret.encode("utf-8")

        child_env = {
            k: v
            for k, v in env.items()
            if (
                k.startswith(("WALLAPOP_", "UPLOAD_"))
                or k in ("PUBLIC_BASE_URL", "PHOTO_URL_HOSTS")
            )
            and k != "WALLAPOP_UPSTREAM_TOKEN"
        }

        return cls(
            upstream_token=upstream_token,
            gateway_registration_token=gateway_registration_token,
            gateway_internal_url=gateway_internal_url,
            host=host,
            port=port,
            advertise_url=advertise_url,
            command=command,
            args=args,
            child_env=child_env,
            upload_dir=upload_dir,
            upload_secret=upload_secret,
            no_register=no_register,
        )
