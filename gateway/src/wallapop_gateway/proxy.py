"""Forward MCP tool calls to one long-lived wallapop-mcp stdio process."""

from collections.abc import AsyncIterator, Mapping, Sequence
from contextlib import asynccontextmanager
from typing import Any

from mcp import types
from mcp.client.session import ClientSession
from mcp.client.stdio import StdioServerParameters, stdio_client
from mcp.server.lowlevel import Server


class Upstream:
    """One child process shared by every HTTP session; it serves requests concurrently."""

    def __init__(self, command: str, args: Sequence[str], env: Mapping[str, str]) -> None:
        self._params = StdioServerParameters(command=command, args=list(args), env=dict(env))
        self._session: ClientSession | None = None
        self.instructions: str | None = None

    @asynccontextmanager
    async def connect(self) -> AsyncIterator[None]:
        async with (
            stdio_client(self._params) as (read, write),
            ClientSession(read, write) as session,
        ):
            init = await session.initialize()
            self.instructions = init.instructions
            self._session = session
            try:
                yield
            finally:
                self._session = None

    def _require(self) -> ClientSession:
        if self._session is None:
            raise RuntimeError("wallapop-mcp is not running")
        return self._session

    async def list_tools(
        self, ctx: Any, params: types.PaginatedRequestParams | None
    ) -> types.ListToolsResult:
        return await self._require().list_tools(params=params)

    async def call_tool(
        self, ctx: Any, params: types.CallToolRequestParams
    ) -> types.CallToolResult:
        result = await self._require().call_tool(params.name, params.arguments or {})
        assert isinstance(result, types.CallToolResult)
        return result


def build_server(upstream: Upstream) -> Server:
    return Server(
        "wallapop-mcp",
        on_list_tools=upstream.list_tools,
        on_call_tool=upstream.call_tool,
    )
