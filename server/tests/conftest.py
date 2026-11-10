"""Shared fixtures and helpers for wallapop-server tests."""

import base64
import hashlib
import hmac
import json
from pathlib import Path
from typing import Any

import pytest

VALID_JPEG = b"\xff\xd8\xff\xe0\x00\x10JFIF\x00\x01\x01\x00\x00\x01\x00\x01\x00\x00\xff\xdb"
VALID_PNG = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01"
VALID_WEBP = b"RIFF\x14\x00\x00\x00WEBPVP8 \x08\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00"

MCP_HEADERS = {
    "Content-Type": "application/json",
    "Accept": "application/json, text/event-stream",
    "MCP-Protocol-Version": "2025-06-18",
}


def sign_ticket(ticket_id: str, exp: int, secret: bytes) -> str:
    """Sign an upload ticket exactly like Rust src/upload.rs:46-54."""
    msg = f"{ticket_id}.{exp}".encode()
    mac = hmac.new(secret, msg, hashlib.sha256).digest()
    return base64.urlsafe_b64encode(mac).rstrip(b"=").decode("utf-8")


def create_meta(upload_dir: Path, ticket_id: str, exp: int, secret: bytes) -> str:
    """Create a .meta file for a ticket and return its signature."""
    sig = sign_ticket(ticket_id, exp, secret)
    meta_path = upload_dir / f"{ticket_id}.meta"
    meta_path.write_text(json.dumps({"id": ticket_id, "exp": exp, "sig": sig}))
    return sig


def rpc_result(response: Any) -> dict[str, Any]:
    """Parse JSON or SSE event stream response into a JSON-RPC dict."""
    if response.headers.get("content-type", "").startswith("text/event-stream"):
        for line in response.text.splitlines():
            if line.startswith("data:"):
                return json.loads(line[5:])
        raise AssertionError(f"no data in event stream: {response.text!r}")
    return response.json()


@pytest.fixture
def fake_upstream_path() -> Path:
    return Path(__file__).parent / "fake_upstream.py"


@pytest.fixture
def upload_secret() -> bytes:
    return b"upload-secret-32-chars-long-abc"


@pytest.fixture
def upstream_token() -> str:
    return "upstream-token-32-chars-long-123"
