"""QR generation, and the generate -> decode round trip."""

import base64
import json

import pytest


def _png(payload):
    """The PNG bytes from the image content block.

    `generate_qr` returns an MCP ImageContent block: `.data` is a base64 string
    and `.mimeType` is "image/png". (`decode`, by contrast, returns TextContent
    whose `.mimeType` is None — do not assert mimeType there.)
    """
    block = payload[0]
    assert block.mimeType == "image/png", f"expected image/png, got {block.mimeType}"
    return base64.b64decode(block.data)


def _b(raw: bytes) -> dict:
    """Wrap raw bytes for a JSON transport — see test_decode.py for why."""
    return {"$bytes": base64.b64encode(raw).decode()}


async def test_generates_a_png(client):
    res = await client.call_tool("generate_qr", {"text": "hello"})
    png = _png(res.content)
    assert png.startswith(b"\x89PNG\r\n\x1a\n"), "not a PNG"


@pytest.mark.parametrize("ecc", ["l", "m", "q", "h"])
@pytest.mark.parametrize(
    "text",
    ["hello", "https://actcore.dev/python", "Привет, мир", "x" * 400],
)
async def test_round_trip(client, ecc, text):
    gen = await client.call_tool("generate_qr", {"text": text, "ecc": ecc})
    png = _png(gen.content)

    dec = await client.call_tool("decode", {"data": _b(png)})
    out = json.loads(dec.content[0].text)
    assert out["count"] == 1
    assert out["results"][0]["format"] == "QR_CODE"
    assert out["results"][0]["text"] == text


async def test_scale_changes_image_size(client):
    small = _png((await client.call_tool("generate_qr", {"text": "x", "scale": 2})).content)
    large = _png((await client.call_tool("generate_qr", {"text": "x", "scale": 16})).content)
    assert len(large) > len(small)
