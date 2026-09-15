"""The component's ceiling and its behaviour on hostile input.

A tool-level failure surfaces through fastmcp's client as `ToolError` (raised
from a `CallToolResult` whose `isError` is true), not as `mcp.shared.
exceptions.McpError` — the latter is a JSON-RPC/protocol-level failure (e.g.
an unknown tool or a malformed request), which none of these cases are: the
component always answers, it just answers with an error. Verified against a
live `act run --mcp` session for every case below.
"""

import asyncio
import base64
import json
from contextlib import AsyncExitStack

import pytest
from fastmcp import Client
from fastmcp.client.transports import StdioTransport
from fastmcp.exceptions import ToolError

from conftest import CONNECT_TIMEOUT, LOG_FILE


def _b(raw: bytes) -> dict:
    """Wrap raw bytes for a JSON transport — see test_decode.py for why."""
    return {"$bytes": base64.b64encode(raw).decode()}


@pytest.fixture
async def ungranted_client(act_command, wasm_path):
    """A client for the same component started with NO capability grants.

    `client` (from conftest.py) always runs with `--allow wasi:filesystem`,
    so it cannot exercise "path outside the grant": under that fixture any
    readable path would be permitted. This fixture is the faithful
    reproduction of the spec's case — headless `ask` with no grant at all,
    which degrades to deny per ACT's capability model. Mirrors conftest.py's
    `client` fixture exactly, minus the grant flags.
    """
    transport = StdioTransport(
        command=act_command[0],
        args=[*act_command[1:], "run", str(wasm_path), "--mcp"],
        keep_alive=False,
        log_file=LOG_FILE,
    )
    async with AsyncExitStack() as stack:
        async with asyncio.timeout(CONNECT_TIMEOUT):
            connected = await stack.enter_async_context(Client(transport))
        yield connected


async def test_blank_image_is_empty_not_an_error(client):
    # A 1x1 white PNG (8-bit grayscale): valid image, no barcode.
    png = bytes.fromhex(
        "89504e470d0a1a0a0000000d49484452000000010000000108000000003a7e9b55"
        "0000000a49444154789c63f80f0001010100b138f6140000000049454e44ae426082"
    )
    res = await client.call_tool("decode", {"data": _b(png)})
    out = json.loads(res.content[0].text)
    assert out["count"] == 0
    assert out["results"] == []


async def test_truncated_image_is_rejected(client):
    with pytest.raises(ToolError):
        await client.call_tool("decode", {"data": _b(b"\x89PNG\r\n\x1a\ntruncated")})


async def test_both_sources_is_an_error(client):
    with pytest.raises(ToolError, match="not both"):
        await client.call_tool("decode", {"data": _b(b"x"), "path": "/etc/hostname"})


async def test_neither_source_is_an_error(client):
    with pytest.raises(ToolError, match="provide the image as"):
        await client.call_tool("decode", {})


async def test_oversized_payload_is_rejected(client):
    with pytest.raises(ToolError, match="Cannot encode as QR"):
        await client.call_tool("generate_qr", {"text": "x" * 10000})


async def test_bad_colour_is_rejected(client):
    with pytest.raises(ToolError, match="Colour must be #rrggbb"):
        await client.call_tool("generate_qr", {"text": "x", "dark": "red"})


async def test_path_outside_grant_is_capability_denied(ungranted_client):
    # `client` (conftest.py) runs with --allow wasi:filesystem, so it cannot
    # exercise this case -- any readable path would be permitted under it.
    # `ungranted_client` starts the same component with no grants at all: the
    # ask-by-default policy degrades to deny in this headless run, so any
    # `path` at all is outside the (empty) grant.
    with pytest.raises(ToolError, match="Permission denied"):
        await ungranted_client.call_tool("decode", {"path": "/etc/hostname"})
