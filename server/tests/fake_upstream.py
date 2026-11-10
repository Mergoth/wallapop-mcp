"""Stand-in for the wallapop-mcp binary: a stdio MCP server with one tool."""

import os

from mcp.server.mcpserver import MCPServer

server = MCPServer("fake-wallapop", instructions="fake upstream instructions")


@server.tool()
def echo(text: str) -> str:
    """Echo the text back, with the session path the gateway passed through."""
    return f"echo:{text}:{os.environ.get('WALLAPOP_SESSION_PATH', '-')}"


@server.tool()
def get_env(name: str) -> str:
    """Return an environment variable, or <missing> if unset."""
    return os.environ.get(name, "<missing>")


if __name__ == "__main__":
    server.run()
