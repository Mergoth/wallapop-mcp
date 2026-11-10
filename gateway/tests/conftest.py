import base64
import hashlib
import json
import secrets
import sys
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any
from urllib.parse import parse_qs, urlparse

import httpx2
import pytest
from starlette.testclient import TestClient

from wallapop_gateway.app import build_app
from wallapop_gateway.config import load_settings

BASE = "http://127.0.0.1:8000"
RESOURCE = f"{BASE}/mcp"
CALLBACK = "https://claude.ai/api/mcp/auth_callback"
GOOGLE_ID = "google-client-id.apps.googleusercontent.com"
OWNER = "owner@example.com"
STATIC = "static-bearer-token-for-claude-code"
MCP_HEADERS = {
    "Content-Type": "application/json",
    "Accept": "application/json, text/event-stream",
    "MCP-Protocol-Version": "2025-06-18",
}


def make_id_token(**overrides: Any) -> str:
    claims = {
        "iss": "https://accounts.google.com",
        "aud": GOOGLE_ID,
        "exp": int(time.time()) + 300,
        "email": OWNER,
        "email_verified": True,
        "sub": "1234567890",
    } | overrides
    claims = {k: v for k, v in claims.items() if v is not None}

    def b64(obj: dict) -> str:
        return base64.urlsafe_b64encode(json.dumps(obj).encode()).decode().rstrip("=")

    return f"{b64({'alg': 'RS256', 'typ': 'JWT'})}.{b64(claims)}.signature"


@dataclass
class FakeGoogle:
    """Google's token endpoint. ``claims`` override the ID token for the next exchange."""

    claims: dict[str, Any] = field(default_factory=dict)
    status: int = 200
    requests: list[dict[str, list[str]]] = field(default_factory=list)
    nonce: str | None = None

    def handler(self, request: httpx2.Request) -> httpx2.Response:
        assert str(request.url) == "https://oauth2.googleapis.com/token"
        form = parse_qs(request.content.decode())
        self.requests.append(form)
        if self.status != 200:
            return httpx2.Response(self.status, json={"error": "invalid_grant"})
        token = make_id_token(**({"nonce": self.nonce} | self.claims))
        return httpx2.Response(200, json={"id_token": token, "access_token": "google-access"})


def env(state_dir: Path, **extra: str) -> dict[str, str]:
    return {
        "MCP_RESOURCE_URL": RESOURCE,
        "GOOGLE_CLIENT_ID": GOOGLE_ID,
        "GOOGLE_CLIENT_SECRET": "google-secret",
        "MCP_OWNER_EMAIL": OWNER,
        "MCP_STATE_DIR": str(state_dir),
        "MCP_AUTH_TOKEN": STATIC,
        "WALLAPOP_MCP_COMMAND": sys.executable,
        "WALLAPOP_MCP_ARGS": str(Path(__file__).with_name("fake_upstream.py")),
        "WALLAPOP_SESSION_PATH": "/data/session.json",
    } | extra


@pytest.fixture
def state_dir(tmp_path: Path) -> Path:
    d = tmp_path / "state"
    d.mkdir()
    return d


@pytest.fixture
def google() -> FakeGoogle:
    return FakeGoogle()


@pytest.fixture
def client(state_dir: Path, google: FakeGoogle):
    http = httpx2.AsyncClient(transport=httpx2.MockTransport(google.handler))
    app = build_app(load_settings(env(state_dir)), http_client=http)
    with TestClient(app, base_url=BASE, follow_redirects=False) as c:
        yield c


def pkce() -> tuple[str, str]:
    verifier = secrets.token_urlsafe(48)
    digest = hashlib.sha256(verifier.encode()).digest()
    return verifier, base64.urlsafe_b64encode(digest).decode().rstrip("=")


def register(c: TestClient, name: str = "Claude", redirect: str = CALLBACK) -> str:
    r = c.post(
        "/register",
        json={
            "client_name": name,
            "redirect_uris": [redirect],
            "token_endpoint_auth_method": "none",
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
        },
    )
    assert r.status_code == 201, r.text
    return r.json()["client_id"]


def authorize(c: TestClient, client_id: str, challenge: str, resource: str = RESOURCE):
    return c.get(
        "/authorize",
        params={
            "response_type": "code",
            "client_id": client_id,
            "redirect_uri": CALLBACK,
            "code_challenge": challenge,
            "code_challenge_method": "S256",
            "state": "st4te",
            "scope": "wallapop",
            "resource": resource,
        },
    )


def login_txn(c: TestClient, client_id: str, challenge: str) -> str:
    r = authorize(c, client_id, challenge)
    assert r.status_code == 302, r.text
    location = r.headers["location"]
    assert location.startswith(f"{BASE}/login?txn=")
    return parse_qs(urlparse(location).query)["txn"][0]


def to_google(c: TestClient, google: FakeGoogle, txn: str) -> dict[str, str]:
    """Open the consent page and click Continue; returns the Google authorize query."""
    assert c.get("/login", params={"txn": txn}).status_code == 200
    r = c.get("/login", params={"txn": txn, "go": "1"})
    assert r.status_code == 302, r.text
    location = urlparse(r.headers["location"])
    assert f"{location.scheme}://{location.netloc}{location.path}" == (
        "https://accounts.google.com/o/oauth2/v2/auth"
    )
    query = {k: v[0] for k, v in parse_qs(location.query).items()}
    google.nonce = query["nonce"]
    return query


def sign_in(c: TestClient, google: FakeGoogle) -> tuple[str, dict]:
    """Full flow as claude.ai drives it; returns (client_id, token response)."""
    client_id = register(c)
    verifier, challenge = pkce()
    txn = login_txn(c, client_id, challenge)
    query = to_google(c, google, txn)
    r = c.get("/oauth/google/callback", params={"state": query["state"], "code": "google-code"})
    assert r.status_code == 302, r.text
    redirect = urlparse(r.headers["location"])
    assert f"{redirect.scheme}://{redirect.netloc}{redirect.path}" == CALLBACK
    params = parse_qs(redirect.query)
    assert params["state"] == ["st4te"]
    r = c.post(
        "/token",
        data={
            "grant_type": "authorization_code",
            "code": params["code"][0],
            "redirect_uri": CALLBACK,
            "client_id": client_id,
            "code_verifier": verifier,
            "resource": RESOURCE,
        },
    )
    assert r.status_code == 200, r.text
    return client_id, r.json()


def rpc_result(r) -> dict:
    if r.headers.get("content-type", "").startswith("text/event-stream"):
        for line in r.text.splitlines():
            if line.startswith("data:"):
                return json.loads(line[5:])
        raise AssertionError(f"no data in event stream: {r.text!r}")
    return r.json()


def mcp_session(c: TestClient, token: str) -> dict[str, str]:
    """Initialize an MCP session; returns the headers for further requests."""
    headers = {**MCP_HEADERS, "Authorization": f"Bearer {token}"}
    r = c.post(
        "/mcp",
        json={
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "test", "version": "0"},
            },
        },
        headers=headers,
    )
    assert r.status_code == 200, r.text
    assert rpc_result(r)["result"]["instructions"] == "fake upstream instructions"
    if session_id := r.headers.get("mcp-session-id"):
        headers["mcp-session-id"] = session_id
    c.post("/mcp", json={"jsonrpc": "2.0", "method": "notifications/initialized"}, headers=headers)
    return headers
