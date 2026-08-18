"""Minimal MCP stdio server built on the official `mcp` SDK, for tests.

Speaks the standard stdio transport: newline-delimited JSON-RPC, no
Content-Length headers.
"""

from mcp.server.fastmcp import FastMCP

server = FastMCP("stdio-test-server")


@server.tool()
def ping(text: str) -> str:
    """Echo the input text."""
    return text


if __name__ == "__main__":
    server.run()  # stdio transport by default
