"""Tests for wallapop-server application and bridge integration."""

import sys
from pathlib import Path
from typing import Any

import pytest
from mcp_gateway import bridge
from starlette.testclient import TestClient

from conftest import MCP_HEADERS, rpc_result
from wallapop_server.app import build_app
from wallapop_server.settings import Settings


def test_mcp_auth_and_echo_flow(
    tmp_path: Path,
    fake_upstream_path: Path,
    upstream_token: str,
) -> None:
    session_file = tmp_path / "session.json"
    session_file.write_text("{}")

    settings = Settings(
        upstream_token=upstream_token,
        gateway_registration_token=None,
        gateway_internal_url="http://mcp-gateway:8081",
        host="127.0.0.1",
        port=8000,
        advertise_url="http://wallapop-mcp:8000/mcp",
        command=sys.executable,
        args=(str(fake_upstream_path.resolve()),),
        child_env={"WALLAPOP_SESSION_PATH": str(session_file)},
        upload_dir=None,
        upload_secret=None,
        no_register=True,
    )

    app = build_app(settings)

    with TestClient(app) as client:
        # 1. /mcp without bearer -> 401
        r_no_auth = client.post("/mcp", headers=MCP_HEADERS, json={})
        assert r_no_auth.status_code == 401

        # 2. /mcp with wrong bearer -> 401
        r_wrong_auth = client.post(
            "/mcp",
            headers={**MCP_HEADERS, "Authorization": "Bearer wrong-token-12345"},
            json={},
        )
        assert r_wrong_auth.status_code == 401

        # 3. With right bearer: initialize
        headers = {**MCP_HEADERS, "Authorization": f"Bearer {upstream_token}"}
        r_init = client.post(
            "/mcp",
            headers=headers,
            json={
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "clientInfo": {"name": "test-client", "version": "1.0"},
                },
            },
        )
        assert r_init.status_code == 200
        init_res = rpc_result(r_init)["result"]
        assert init_res["instructions"] == "fake upstream instructions"

        if session_id := r_init.headers.get("mcp-session-id"):
            headers["mcp-session-id"] = session_id

        # Send notifications/initialized
        client.post(
            "/mcp",
            headers=headers,
            json={"jsonrpc": "2.0", "method": "notifications/initialized"},
        )

        # 4. tools/call echo
        r_echo = client.post(
            "/mcp",
            headers=headers,
            json={
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": {"name": "echo", "arguments": {"text": "hello"}},
            },
        )
        assert r_echo.status_code == 200
        content = rpc_result(r_echo)["result"]["content"]
        assert content[0]["text"] == f"echo:hello:{session_file}"

        # 5. Child never sees WALLAPOP_UPSTREAM_TOKEN
        r_env = client.post(
            "/mcp",
            headers=headers,
            json={
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": {
                    "name": "get_env",
                    "arguments": {"name": "WALLAPOP_UPSTREAM_TOKEN"},
                },
            },
        )
        assert r_env.status_code == 200
        env_content = rpc_result(r_env)["result"]["content"]
        assert env_content[0]["text"] == "<missing>"


def test_upload_route_exists_only_when_upload_dir_is_set(
    tmp_path: Path,
    fake_upstream_path: Path,
    upstream_token: str,
) -> None:
    upload_dir = tmp_path / "uploads"
    upload_dir.mkdir()

    # App with upload_dir set
    settings_with_upload = Settings(
        upstream_token=upstream_token,
        gateway_registration_token=None,
        gateway_internal_url="http://mcp-gateway:8081",
        host="127.0.0.1",
        port=8000,
        advertise_url="http://wallapop-mcp:8000/mcp",
        command=sys.executable,
        args=(str(fake_upstream_path.resolve()),),
        child_env={},
        upload_dir=upload_dir,
        upload_secret=b"secret",
        no_register=True,
    )
    app_with_upload = build_app(settings_with_upload)

    with TestClient(app_with_upload) as client:
        # Route exists: bad ticket id returns 400 (from handle_upload)
        r = client.put("/upload/bad_ticket")
        assert r.status_code == 400
        assert r.json() == {"error": "invalid_ticket_id"}

    # App without upload_dir
    settings_without_upload = Settings(
        upstream_token=upstream_token,
        gateway_registration_token=None,
        gateway_internal_url="http://mcp-gateway:8081",
        host="127.0.0.1",
        port=8000,
        advertise_url="http://wallapop-mcp:8000/mcp",
        command=sys.executable,
        args=(str(fake_upstream_path.resolve()),),
        child_env={},
        upload_dir=None,
        upload_secret=None,
        no_register=True,
    )
    app_without_upload = build_app(settings_without_upload)

    with TestClient(app_without_upload) as client:
        # Route does not exist -> 404
        r = client.put("/upload/bad_ticket")
        assert r.status_code == 404


def test_registration_spec(
    tmp_path: Path,
    upstream_token: str,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    captured_registrations: list[Any] = []
    real_build_app = bridge.build_app

    def spy_build_app(*args: Any, **kwargs: Any) -> Any:
        captured_registrations.append(kwargs.get("registration"))
        return real_build_app(*args, **kwargs)

    monkeypatch.setattr(bridge, "build_app", spy_build_app)

    # 1. With upload enabled
    upload_dir = tmp_path / "uploads"
    settings_upload = Settings(
        upstream_token=upstream_token,
        gateway_registration_token="reg-token-32-chars-long-12345",
        gateway_internal_url="http://mcp-gateway:8081",
        host="127.0.0.1",
        port=8000,
        advertise_url="http://wallapop-mcp:8000/mcp",
        command="wallapop-mcp",
        args=(),
        child_env={},
        upload_dir=upload_dir,
        upload_secret=b"secret",
        no_register=False,
    )
    build_app(settings_upload)
    reg = captured_registrations.pop()
    assert reg is not None
    _client, internal_url, token, spec = reg
    assert internal_url == "http://mcp-gateway:8081"
    assert token == "reg-token-32-chars-long-12345"
    assert spec == {
        "name": "wallapop",
        "url": "http://wallapop-mcp:8000/mcp",
        "title": "Wallapop",
        "upstream_token": upstream_token,
        "public_paths": ["/upload/"],
    }

    # 2. With upload disabled (no public_paths)
    settings_no_upload = Settings(
        upstream_token=upstream_token,
        gateway_registration_token="reg-token-32-chars-long-12345",
        gateway_internal_url="http://mcp-gateway:8081",
        host="127.0.0.1",
        port=8000,
        advertise_url="http://wallapop-mcp:8000/mcp",
        command="wallapop-mcp",
        args=(),
        child_env={},
        upload_dir=None,
        upload_secret=None,
        no_register=False,
    )
    build_app(settings_no_upload)
    reg2 = captured_registrations.pop()
    assert reg2 is not None
    _, _, _, spec2 = reg2
    assert spec2 == {
        "name": "wallapop",
        "url": "http://wallapop-mcp:8000/mcp",
        "title": "Wallapop",
        "upstream_token": upstream_token,
    }
    assert "public_paths" not in spec2

    # 3. With WALLAPOP_NO_REGISTER=1 -> registration is None
    settings_no_register = Settings(
        upstream_token=upstream_token,
        gateway_registration_token=None,
        gateway_internal_url="http://mcp-gateway:8081",
        host="127.0.0.1",
        port=8000,
        advertise_url="http://wallapop-mcp:8000/mcp",
        command="wallapop-mcp",
        args=(),
        child_env={},
        upload_dir=None,
        upload_secret=None,
        no_register=True,
    )
    build_app(settings_no_register)
    reg3 = captured_registrations.pop()
    assert reg3 is None
