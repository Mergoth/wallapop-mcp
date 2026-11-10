"""Starlette app for wallapop-server behind the shared gateway."""

import asyncio
import contextlib
from collections.abc import AsyncIterator
from contextlib import asynccontextmanager
from typing import Any

import httpx2
from mcp_gateway import bridge
from starlette.applications import Starlette
from starlette.routing import Route

from wallapop_server.settings import Settings
from wallapop_server.upload import _gc_loop, handle_upload


def build_app(
    settings: Settings,
    *,
    registration_client: httpx2.AsyncClient | None = None,
) -> Starlette:
    extra_routes: list[Route] = []
    if settings.upload_dir is not None:
        extra_routes.append(Route("/upload/{ticket_id}", handle_upload, methods=["PUT"]))

    reg_client: httpx2.AsyncClient | None = None
    registration: tuple[httpx2.AsyncClient, str, str, dict[str, Any]] | None = None

    if not settings.no_register:
        assert settings.gateway_registration_token is not None
        reg_client = registration_client or httpx2.AsyncClient()
        spec: dict[str, Any] = {
            "name": "wallapop",
            "url": settings.advertise_url,
            "title": "Wallapop",
            "upstream_token": settings.upstream_token,
        }
        if settings.upload_dir is not None:
            spec["public_paths"] = ["/upload/"]
        registration = (
            reg_client,
            settings.gateway_internal_url,
            settings.gateway_registration_token,
            spec,
        )

    app = bridge.build_app(
        command=settings.command,
        args=settings.args,
        env=settings.child_env,
        name="wallapop",
        bearer_token=settings.upstream_token,
        extra_routes=extra_routes,
        registration=registration,
    )

    if settings.upload_dir is not None:
        assert settings.upload_secret is not None
        app.state.upload_dir = settings.upload_dir
        app.state.upload_secret = settings.upload_secret

    bridge_lifespan = app.router.lifespan_context

    @asynccontextmanager
    async def lifespan(a: Starlette) -> AsyncIterator[None]:
        gc_task: asyncio.Task[None] | None = None
        if settings.upload_dir is not None:
            settings.upload_dir.mkdir(parents=True, exist_ok=True)
            gc_task = asyncio.create_task(_gc_loop(settings.upload_dir))
        try:
            async with bridge_lifespan(a):
                yield
        finally:
            if gc_task is not None:
                gc_task.cancel()
                with contextlib.suppress(asyncio.CancelledError):
                    await gc_task
            if registration_client is None and reg_client is not None:
                await reg_client.aclose()

    app.router.lifespan_context = lifespan
    return app
