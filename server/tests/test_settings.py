"""Tests for Settings.from_env configuration parsing."""

from pathlib import Path

import pytest

from wallapop_server.settings import ConfigError, Settings


def test_token_missing() -> None:
    env = {
        "GATEWAY_REGISTRATION_TOKEN": "r" * 32,
    }
    with pytest.raises(ConfigError) as exc_info:
        Settings.from_env(env)
    assert "WALLAPOP_UPSTREAM_TOKEN" in str(exc_info.value)


def test_token_too_short() -> None:
    env = {
        "WALLAPOP_UPSTREAM_TOKEN": "short",
        "GATEWAY_REGISTRATION_TOKEN": "r" * 32,
    }
    with pytest.raises(ConfigError) as exc_info:
        Settings.from_env(env)
    assert "WALLAPOP_UPSTREAM_TOKEN" in str(exc_info.value)


def test_registration_token_missing() -> None:
    env = {
        "WALLAPOP_UPSTREAM_TOKEN": "u" * 32,
    }
    with pytest.raises(ConfigError) as exc_info:
        Settings.from_env(env)
    assert "GATEWAY_REGISTRATION_TOKEN" in str(exc_info.value)


def test_registration_token_missing_allowed_with_no_register() -> None:
    env = {
        "WALLAPOP_UPSTREAM_TOKEN": "u" * 32,
        "WALLAPOP_NO_REGISTER": "1",
    }
    settings = Settings.from_env(env)
    assert settings.no_register is True
    assert settings.gateway_registration_token is None


def test_upload_dir_without_secret() -> None:
    env = {
        "WALLAPOP_UPSTREAM_TOKEN": "u" * 32,
        "WALLAPOP_NO_REGISTER": "1",
        "UPLOAD_DIR": "/data/uploads",
    }
    with pytest.raises(ConfigError) as exc_info:
        Settings.from_env(env)
    assert "UPLOAD_SECRET" in str(exc_info.value)


def test_upload_dir_with_secret() -> None:
    env = {
        "WALLAPOP_UPSTREAM_TOKEN": "u" * 32,
        "WALLAPOP_NO_REGISTER": "1",
        "UPLOAD_DIR": "/data/uploads",
        "UPLOAD_SECRET": "mysecret",
    }
    settings = Settings.from_env(env)
    assert settings.upload_dir == Path("/data/uploads")
    assert settings.upload_secret == b"mysecret"


def test_child_env_filtering() -> None:
    env = {
        "WALLAPOP_UPSTREAM_TOKEN": "u" * 32,
        "GATEWAY_REGISTRATION_TOKEN": "r" * 32,
        "WALLAPOP_SESSION_PATH": "/data/session.json",
        "WALLAPOP_CUSTOM_OPTION": "yes",
        "UPLOAD_DIR": "/data/uploads",
        "UPLOAD_SECRET": "secret",
        "UPLOAD_TTL_SECONDS": "600",
        "PUBLIC_BASE_URL": "https://mcp.mergoth.synology.me/wallapop",
        "PHOTO_URL_HOSTS": "example.com",
        "PATH": "/usr/bin:/bin",
        "OTHER_UNRELATED_VAR": "leave_out",
    }
    settings = Settings.from_env(env)

    # Must contain WALLAPOP_*, UPLOAD_*, PUBLIC_BASE_URL, PHOTO_URL_HOSTS
    assert settings.child_env["WALLAPOP_SESSION_PATH"] == "/data/session.json"
    assert settings.child_env["WALLAPOP_CUSTOM_OPTION"] == "yes"
    assert settings.child_env["UPLOAD_DIR"] == "/data/uploads"
    assert settings.child_env["UPLOAD_SECRET"] == "secret"
    assert settings.child_env["UPLOAD_TTL_SECONDS"] == "600"
    assert settings.child_env["PUBLIC_BASE_URL"] == "https://mcp.mergoth.synology.me/wallapop"
    assert settings.child_env["PHOTO_URL_HOSTS"] == "example.com"

    # Must NOT contain WALLAPOP_UPSTREAM_TOKEN or PATH
    assert "WALLAPOP_UPSTREAM_TOKEN" not in settings.child_env
    assert "PATH" not in settings.child_env
    assert "OTHER_UNRELATED_VAR" not in settings.child_env
    assert "GATEWAY_REGISTRATION_TOKEN" not in settings.child_env


def test_defaults_and_args_parsing() -> None:
    env = {
        "WALLAPOP_UPSTREAM_TOKEN": "u" * 32,
        "GATEWAY_REGISTRATION_TOKEN": "r" * 32,
        "WALLAPOP_MCP_ARGS": "--allow-writes --option 'val with spaces'",
    }
    settings = Settings.from_env(env)
    assert settings.gateway_internal_url == "http://mcp-gateway:8081"
    assert settings.host == "127.0.0.1"
    assert settings.port == 8000
    assert settings.advertise_url == "http://wallapop-mcp:8000/mcp"
    assert settings.command == "wallapop-mcp"
    assert settings.args == ("--allow-writes", "--option", "val with spaces")
    assert settings.upload_dir is None
    assert settings.upload_secret is None


def test_invalid_port() -> None:
    env = {
        "WALLAPOP_UPSTREAM_TOKEN": "u" * 32,
        "GATEWAY_REGISTRATION_TOKEN": "r" * 32,
        "WALLAPOP_PORT": "invalid_port",
    }
    with pytest.raises(ConfigError) as exc_info:
        Settings.from_env(env)
    assert "WALLAPOP_PORT" in str(exc_info.value)
