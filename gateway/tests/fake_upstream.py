"""Stand-in for the wallapop-mcp binary: a stdio MCP server with one tool."""

import os

from mcp.server.mcpserver import MCPServer

server = MCPServer("fake-wallapop", instructions="fake upstream instructions")


@server.tool()
def echo(text: str) -> str:
    """Echo the text back, with the session path the gateway passed through."""
    return f"echo:{text}:{os.environ.get('WALLAPOP_SESSION_PATH', '-')}"


if __name__ == "__main__":
    server.run()
