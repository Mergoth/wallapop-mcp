"""Assemble the Starlette app: SDK auth routes, Google sign-in routes and the proxied /mcp."""

import asyncio
import logging
import time
from collections.abc import AsyncIterator
from contextlib import asynccontextmanager
from pathlib import Path

import httpx2
from mcp.server.auth.provider import ProviderTokenVerifier
from starlette.applications import Starlette
from starlette.routing import Route

from wallapop_gateway.config import Settings
from wallapop_gateway.metadata import build_auth_settings, enable_public_client_metadata
from wallapop_gateway.oauth import CALLBACK_PATH, LOGIN_PATH, GoogleOwnerOAuthProvider
from wallapop_gateway.proxy import Upstream, build_server
from wallapop_gateway.upload import handle_upload

logger = logging.getLogger(__name__)

_GC_INTERVAL = 600  # seconds between GC sweeps
_GC_MAX_AGE = 3600  # files older than 1 h are eligible


async def _gc_once(upload_dir: Path) -> None:
    if not upload_dir.exists():
        return
    cutoff = time.time() - _GC_MAX_AGE
    for path in upload_dir.iterdir():
        if path.suffix in {".meta", ".bin", ".consumed"}:
            try:
                if path.stat().st_mtime < cutoff:
                    path.unlink(missing_ok=True)
            except OSError:
                pass


async def _gc_loop(upload_dir: Path) -> None:
    while True:
        try:
            await _gc_once(upload_dir)
        except Exception:
            logger.exception("upload GC error")
        await asyncio.sleep(_GC_INTERVAL)


def build_app(settings: Settings, *, http_client: httpx2.AsyncClient | None = None) -> Starlette:
    enable_public_client_metadata()
    provider = GoogleOwnerOAuthProvider(
        google_client_id=settings.google_client_id,
        google_client_secret=settings.google_client_secret,
        owner_email=settings.owner_email,
        static_token=settings.static_token,
        resource_url=settings.resource_url,
        issuer_url=settings.issuer_url,
        state_dir=settings.state_dir,
        http_client=http_client,
    )
    upstream = Upstream(settings.upstream_command, settings.upstream_args, settings.upstream_env)
    server = build_server(upstream)

    extra_routes = [
        Route(LOGIN_PATH, provider.login, methods=["GET", "POST"]),
        Route(CALLBACK_PATH, provider.google_callback, methods=["GET"]),
    ]
    if settings.upload_dir is not None:
        extra_routes.append(Route("/upload/{ticket_id}", handle_upload, methods=["PUT"]))

    app = server.streamable_http_app(
        host=settings.host,
        auth=build_auth_settings(
            resource_url=settings.resource_url, issuer_url=settings.issuer_url
        ),
        token_verifier=ProviderTokenVerifier(provider),
        auth_server_provider=provider,
        custom_starlette_routes=extra_routes,
    )

    # Attach upload state so the handler can reach it.
    if settings.upload_dir is not None:
        app.state.upload_dir = settings.upload_dir
        app.state.upload_secret = settings.upload_secret

    session_manager_lifespan = app.router.lifespan_context

    @asynccontextmanager
    async def lifespan(a: Starlette) -> AsyncIterator[None]:
        if settings.upload_dir is not None:
            settings.upload_dir.mkdir(parents=True, exist_ok=True)
            asyncio.ensure_future(_gc_loop(settings.upload_dir))
        async with upstream.connect():
            server.instructions = upstream.instructions
            async with session_manager_lifespan(a):
                yield

    app.router.lifespan_context = lifespan
    return app
