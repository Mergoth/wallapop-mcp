"""Tests proxying through the real mcp-gateway app."""

import asyncio
import sys
import time
from pathlib import Path

import httpx2
from mcp_gateway.app import build_app as build_gateway_app
from mcp_gateway.backends import Backend, BackendTable
from mcp_gateway.config import Settings as GatewaySettings

from conftest import VALID_JPEG, create_meta
from wallapop_server.app import build_app as build_server_app
from wallapop_server.settings import Settings


def test_through_the_gateway(
    tmp_path: Path,
    fake_upstream_path: Path,
    upstream_token: str,
    upload_secret: bytes,
) -> None:
    async def _test() -> None:
        upload_dir = tmp_path / "uploads"
        upload_dir.mkdir()
        state_dir = tmp_path / "gateway_state"
        state_dir.mkdir()

        # Build the real wallapop server app
        server_settings = Settings(
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
            upload_secret=upload_secret,
            no_register=True,
        )
        server_app = build_server_app(server_settings)

        # Wire proxy_client via ASGITransport to the server app
        proxy_client = httpx2.AsyncClient(
            transport=httpx2.ASGITransport(app=server_app),
            base_url="http://wallapop-mcp:8000",
        )

        # Register wallapop backend in BackendTable with public_paths: ("/upload/",)
        table = BackendTable()
        table.add(
            Backend(
                name="wallapop",
                url="http://wallapop-mcp:8000/mcp",
                title="Wallapop",
                upstream_token=upstream_token,
                public_paths=("/upload/",),
            )
        )

        gw_settings = GatewaySettings(
            host="127.0.0.1",
            port=8080,
            base_url="https://mcp.mergoth.synology.me",
            google_client_id="dummy-google-client",
            google_client_secret="dummy-google-secret",
            allowed_emails=frozenset(["owner@example.com"]),
            state_dir=state_dir,
            registration_token="r" * 32,
        )
        gw_app = build_gateway_app(gw_settings, backends=table, proxy_client=proxy_client)

        ticket_id = "0123456789abcdef"
        exp = int(time.time()) + 300
        sig = create_meta(upload_dir, ticket_id, exp, upload_secret)

        async with httpx2.AsyncClient(
            transport=httpx2.ASGITransport(app=gw_app),
            base_url="https://mcp.mergoth.synology.me",
        ) as client:
            # 1. PUT /wallapop/upload/<id>?exp&sig with JPEG and no Authorization -> 201
            resp_upload = await client.put(
                f"/wallapop/upload/{ticket_id}?exp={exp}&sig={sig}",
                content=VALID_JPEG,
            )
            assert resp_upload.status_code == 201
            assert resp_upload.json() == {
                "ref": f"upload:{ticket_id}",
                "bytes": len(VALID_JPEG),
            }

            # File lands on disk
            bin_path = upload_dir / f"{ticket_id}.bin"
            assert bin_path.exists()
            assert bin_path.read_bytes() == VALID_JPEG

            # 2. POST /wallapop/mcp without a bearer is 401 from the gateway
            resp_mcp = await client.post("/wallapop/mcp")
            assert resp_mcp.status_code == 401
            assert "Bearer" in resp_mcp.headers.get("www-authenticate", "")

        # 3. PUT /wallapop/upload/../mcp is 404 and the child saw nothing
        status_code: int | None = None

        async def send(message: dict) -> None:
            nonlocal status_code
            if message["type"] == "http.response.start":
                status_code = message["status"]

        async def receive() -> dict:
            return {"type": "http.request", "body": b"", "more_body": False}

        scope = {
            "type": "http",
            "method": "PUT",
            "path": "/wallapop/upload/../mcp",
            "raw_path": b"/wallapop/upload/../mcp",
            "headers": [],
        }
        await gw_app(scope, receive, send)
        assert status_code == 404

        await proxy_client.aclose()

    asyncio.run(_test())
