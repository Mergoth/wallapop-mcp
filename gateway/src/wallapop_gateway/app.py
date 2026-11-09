"""Assemble the Starlette app: SDK auth routes, Google sign-in routes and the proxied /mcp."""

import base64
import logging
from collections.abc import AsyncIterator
from contextlib import asynccontextmanager
from typing import Callable
from urllib.parse import parse_qs, quote, unquote, urlencode

import httpx2
from mcp.server.auth.provider import ProviderTokenVerifier
from starlette.applications import Starlette
from starlette.middleware.base import BaseHTTPMiddleware
from starlette.requests import Request
from starlette.responses import Response
from starlette.routing import Route
from starlette.types import ASGIApp, Message, Receive, Scope, Send

from wallapop_gateway.config import Settings
from wallapop_gateway.metadata import build_auth_settings, enable_public_client_metadata
from wallapop_gateway.oauth import CALLBACK_PATH, LOGIN_PATH, GoogleOwnerOAuthProvider
from wallapop_gateway.proxy import Upstream, build_server

_log = logging.getLogger("wallapop_gateway.token")


class _McpDiscoveryAlias(BaseHTTPMiddleware):
    """Alias /mcp/.well-known/* → /.well-known/* for clients that path-append the MCP URL.

    Some clients (including ChatGPT) discover OAuth metadata by appending the MCP resource
    path to the well-known prefix, e.g. /.well-known/oauth-authorization-server/mcp.
    The MCP SDK only registers the standard root-level paths; this middleware rewrites the
    path before routing so both forms work.
    """

    async def dispatch(self, request: Request, call_next: Callable) -> Response:
        path = request.url.path
        if path.startswith("/mcp/.well-known/"):
            request.scope["path"] = path[len("/mcp"):]
        elif path == "/.well-known/openid-configuration":
            # ChatGPT requests OIDC discovery; alias it to our OAuth 2.0 metadata.
            request.scope["path"] = "/.well-known/oauth-authorization-server"
        return await call_next(request)


def _parse_basic(header: str) -> tuple[str, str] | None:
    if not header.startswith("Basic "):
        return None
    try:
        decoded = base64.b64decode(header[6:], validate=True).decode("utf-8")
    except (ValueError, UnicodeDecodeError):
        return None
    if ":" not in decoded:
        return None
    client_id, secret = decoded.split(":", 1)
    return unquote(client_id), unquote(secret)


class _TokenClientAuthCompat:
    """Accept a client secret at /token in the form or as HTTP Basic, whichever the client sends.

    The SDK only accepts the method a client registered, and requires client_id in the form
    even for Basic. Clients we pre-register by hand (Gemini app/Enterprise) may use either.
    """

    def __init__(self, app: ASGIApp, provider: GoogleOwnerOAuthProvider) -> None:
        self.app = app
        self._provider = provider

    async def __call__(self, scope: Scope, receive: Receive, send: Send) -> None:
        if scope["type"] != "http" or scope["path"] != "/token" or scope["method"] != "POST":
            await self.app(scope, receive, send)
            return
        body = b""
        more = True
        while more:
            message = await receive()
            body += message.get("body", b"")
            more = message.get("more_body", False)

        headers = list(scope["headers"])
        form = parse_qs(body.decode("utf-8", "replace"), keep_blank_values=True)
        auth = next((v.decode("latin-1") for k, v in headers if k == b"authorization"), "")
        basic = _parse_basic(auth)
        client_id = (form.get("client_id") or [""])[0] or (basic[0] if basic else "")
        client = await self._provider.get_client(client_id) if client_id else None
        if client is not None:
            secret = basic[1] if basic else (form.get("client_secret") or [""])[0]
            form["client_id"] = [client_id]
            method = client.token_endpoint_auth_method
            if method == "client_secret_post" and secret:
                form["client_secret"] = [secret]
            elif method == "client_secret_basic" and secret and basic is None:
                creds = f"{quote(client_id, safe='')}:{quote(secret, safe='')}"
                headers.append((b"authorization", b"Basic " + base64.b64encode(creds.encode())))
            body = urlencode(form, doseq=True).encode()
            headers = [(k, v) for k, v in headers if k != b"content-length"]
            headers.append((b"content-length", str(len(body)).encode()))
            scope = {**scope, "headers": headers}

        replayed = False

        async def replay() -> Message:
            nonlocal replayed
            if not replayed:
                replayed = True
                return {"type": "http.request", "body": body, "more_body": False}
            return await receive()

        await self.app(scope, replay, send)


class _TokenRequestLog(BaseHTTPMiddleware):
    """Log the shape of each /token call and the SDK's error body; never codes or secrets."""

    def __init__(self, app, provider: GoogleOwnerOAuthProvider) -> None:
        super().__init__(app)
        self._provider = provider

    async def dispatch(self, request: Request, call_next: Callable) -> Response:
        if request.url.path != "/token" or request.method != "POST":
            return await call_next(request)
        form = parse_qs((await request.body()).decode("utf-8", "replace"))
        client_id = (form.get("client_id") or [""])[0]
        known = bool(client_id) and await self._provider.get_client(client_id) is not None
        auth = request.headers.get("authorization", "")
        response = await call_next(request)
        body = b"".join([chunk async for chunk in response.body_iterator])
        _log.warning(
            "POST /token grant=%s client_id=%s known=%s form_keys=%s auth_header=%s "
            "content_type=%s -> %d %s",
            (form.get("grant_type") or [""])[0],
            client_id or "<missing>",
            known,
            sorted(form),
            auth.split(" ", 1)[0] if auth else "<none>",
            request.headers.get("content-type", ""),
            response.status_code,
            body[:300].decode("utf-8", "replace") if response.status_code >= 400 else "",
        )
        return Response(
            body,
            status_code=response.status_code,
            headers=dict(response.headers),
            media_type=response.media_type,
        )


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
    app.add_middleware(_TokenClientAuthCompat, provider=provider)
    app.add_middleware(_TokenRequestLog, provider=provider)
    app.add_middleware(_McpDiscoveryAlias)
    return app
