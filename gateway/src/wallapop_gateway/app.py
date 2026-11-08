"""Assemble the Starlette app: SDK auth routes, Google sign-in routes and the proxied /mcp."""

from collections.abc import AsyncIterator
from contextlib import asynccontextmanager

import httpx2
from mcp.server.auth.provider import ProviderTokenVerifier
from starlette.applications import Starlette
from starlette.routing import Route

from wallapop_gateway.config import Settings
from wallapop_gateway.metadata import build_auth_settings, enable_public_client_metadata
from wallapop_gateway.oauth import CALLBACK_PATH, LOGIN_PATH, GoogleOwnerOAuthProvider
from wallapop_gateway.proxy import Upstream, build_server


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
    app = server.streamable_http_app(
        host=settings.host,
        auth=build_auth_settings(
            resource_url=settings.resource_url, issuer_url=settings.issuer_url
        ),
        token_verifier=ProviderTokenVerifier(provider),
        auth_server_provider=provider,
        custom_starlette_routes=[
            Route(LOGIN_PATH, provider.login, methods=["GET", "POST"]),
            Route(CALLBACK_PATH, provider.google_callback, methods=["GET"]),
        ],
    )

    session_manager_lifespan = app.router.lifespan_context

    @asynccontextmanager
    async def lifespan(a: Starlette) -> AsyncIterator[None]:
        async with upstream.connect():
            server.instructions = upstream.instructions
            async with session_manager_lifespan(a):
                yield

    app.router.lifespan_context = lifespan
    return app
